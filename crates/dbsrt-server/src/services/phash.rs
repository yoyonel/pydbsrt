//! Visual perceptual hash extraction, video probing, and temporal shift calculations.

use crate::constants::{
    DEFAULT_BASELINE_FPS, DEFAULT_CUT_THRESHOLD, DEFAULT_FALLBACK_FPS,
    MAX_CORRELATION_SAMPLE_FRAMES, MILLIS_PER_SEC, MIN_CUTS_FOR_CONSENSUS,
    MIN_CUT_CONSENSUS_CONFIDENCE, PERCENT_SCALE, PHASH_BIT_DEPTH, REFINEMENT_STEP_RADIUS,
};
use anyhow::Result;
use dbsrt_core::{
    detect_cuts, extract_frames_phash, probe_video, read_phash_file, write_phash_file, CutItem,
};
use dbsrt_index::{parse_fps_from_phash_stem, BkItem, BkTree};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub type ExistingPhashData = (PathBuf, Vec<u64>, f64);

/// Normalizes a string by keeping only lowercase alphanumeric characters.
pub fn norm_alnum(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Parses perceptual hashes from hex or decimal string representations.
pub fn parse_fingerprint_hashes(fingerprint: &[String]) -> Vec<u64> {
    fingerprint
        .iter()
        .filter_map(|s| {
            let s_trim = s.trim();
            if let Some(hex_part) = s_trim
                .strip_prefix("0x")
                .or_else(|| s_trim.strip_prefix("0X"))
            {
                u64::from_str_radix(hex_part, 16).ok()
            } else if s_trim.chars().any(|c| matches!(c, 'a'..='f' | 'A'..='F')) {
                u64::from_str_radix(s_trim, 16).ok()
            } else {
                s_trim
                    .parse::<u64>()
                    .ok()
                    .or_else(|| u64::from_str_radix(s_trim, 16).ok())
            }
        })
        .collect()
}

pub fn parse_phash_fps(name: &str, media_file: &Path) -> f64 {
    let mut fps = if let Some(fps_match) = name.strip_suffix("fps.phash") {
        let parts: Vec<&str> = fps_match.split('.').collect();
        if parts.len() >= 2 {
            let last_two = format!("{}.{}", parts[parts.len() - 2], parts[parts.len() - 1]);
            if let Ok(v) = last_two.parse::<f64>() {
                v
            } else {
                parts
                    .last()
                    .and_then(|p| p.parse::<f64>().ok())
                    .unwrap_or(25.0)
            }
        } else {
            parts
                .last()
                .and_then(|p| p.parse::<f64>().ok())
                .unwrap_or(25.0)
        }
    } else {
        probe_video(media_file).map(|m| m.fps).unwrap_or(25.0)
    };
    if fps <= 0.0 || fps.is_nan() {
        fps = 25.0;
    }
    fps
}

pub fn find_existing_phash(media_file: &Path, phash_dir: &Path) -> Option<ExistingPhashData> {
    let full_name = media_file
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let stem = media_file
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    if media_file.extension().and_then(|e| e.to_str()) == Some("phash") && media_file.exists() {
        if let Ok(hashes) = read_phash_file(media_file) {
            let fps = parse_fps_from_phash_stem(&stem);
            return Some((media_file.to_path_buf(), hashes, fps));
        }
    }

    if !phash_dir.exists() {
        return None;
    }
    let entries: Vec<_> = std::fs::read_dir(phash_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();

    // Pass 1: Prefer exact full filename match
    if !full_name.is_empty() {
        for p in &entries {
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if name.starts_with(&full_name) && name.ends_with(".phash") {
                    if let Ok(hashes) = read_phash_file(p) {
                        let fps = parse_phash_fps(name, media_file);
                        return Some((p.clone(), hashes, fps));
                    }
                }
            }
        }
    }

    // Pass 2: Fallback to stem match ONLY if container extension matches
    let known_video_exts = [".mp4.", ".webm.", ".mkv.", ".avi.", ".mov."];
    let my_ext = media_file
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}.", e.to_lowercase()));

    if !stem.is_empty() {
        for p in &entries {
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if name.contains(&stem) && name.ends_with(".phash") {
                    let name_lower = name.to_lowercase();
                    let has_conflicting_ext = known_video_exts
                        .iter()
                        .any(|ext| name_lower.contains(ext) && my_ext.as_deref() != Some(*ext));
                    if has_conflicting_ext {
                        continue;
                    }
                    if let Ok(hashes) = read_phash_file(p) {
                        let fps = parse_phash_fps(name, media_file);
                        return Some((p.clone(), hashes, fps));
                    }
                }
            }
        }
    }
    None
}

pub fn resolve_or_extract_phash(media_file: &Path, phash_dir: &Path) -> Result<ExistingPhashData> {
    if let Some(existing) = find_existing_phash(media_file, phash_dir) {
        return Ok(existing);
    }

    let meta = probe_video(media_file)?;
    let total_frames = (meta.duration * meta.fps).floor() as usize;
    let hashes = extract_frames_phash(media_file, 0.0, total_frames)?;

    let out_name = format!(
        "{}.{:.1}fps.phash",
        media_file.file_name().unwrap().to_string_lossy(),
        meta.fps
    );
    let out_path = phash_dir.join(&out_name);
    write_phash_file(&out_path, &hashes)?;

    Ok((out_path, hashes, meta.fps))
}

pub fn read_ref_phash_for_media(phash_dir: &Path, media_name: &str) -> Result<Vec<u64>> {
    let clean_name = media_name.strip_suffix(".phash").unwrap_or(media_name);
    let entries = std::fs::read_dir(phash_dir)?;
    let mut candidate_paths = Vec::new();
    let norm_target = norm_alnum(clean_name);

    for entry in entries.flatten() {
        let p = entry.path();
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            if name.ends_with(".phash") && !name.contains("_mid15s") {
                let cand_stem = name.trim_end_matches(".phash");
                if name == format!("{}.phash", clean_name) || name == clean_name {
                    return read_phash_file(&p);
                }
                let norm_cand = norm_alnum(cand_stem);
                if name.contains(clean_name)
                    || clean_name.contains(cand_stem)
                    || (!norm_cand.is_empty()
                        && (norm_cand.contains(&norm_target) || norm_target.contains(&norm_cand)))
                {
                    candidate_paths.push(p);
                }
            }
        }
    }

    if let Some(first_match) = candidate_paths.first() {
        return read_phash_file(first_match);
    }

    Err(anyhow::anyhow!(
        "No matching reference .phash file found for {:?} in {:?}",
        media_name,
        phash_dir
    ))
}

pub fn extract_media_query_sample(
    media_path: &Path,
    phash_dirs: &[PathBuf],
    nb_seconds: f64,
) -> (f64, Vec<u64>, Option<ExistingPhashData>) {
    let existing_phash = phash_dirs
        .iter()
        .find_map(|dir| find_existing_phash(media_path, dir));

    let (target_fps, query_sample) = if let Some((_, ref hashes, fps)) = existing_phash {
        let sample_len = ((nb_seconds * fps).round() as usize).min(hashes.len());
        let center_start = if hashes.len() > sample_len {
            (hashes.len() / 2).saturating_sub(sample_len / 2)
        } else {
            0
        };

        let nominal_sample = &hashes[center_start..center_start + sample_len];
        let nominal_unique = nominal_sample
            .iter()
            .copied()
            .filter(|&h| !dbsrt_core::is_blank_phash(h))
            .collect::<HashSet<_>>()
            .len();

        let chosen_start = if nominal_unique >= 2 || hashes.len() <= sample_len {
            center_start
        } else {
            let step = (fps.round() as usize / 2).max(1);
            let mut best_start = center_start;
            let mut best_unique = nominal_unique;

            let mut cur = 0;
            while cur + sample_len <= hashes.len() {
                let win = &hashes[cur..cur + sample_len];
                let uniq = win
                    .iter()
                    .copied()
                    .filter(|&h| !dbsrt_core::is_blank_phash(h))
                    .collect::<HashSet<_>>()
                    .len();
                if uniq > best_unique {
                    best_unique = uniq;
                    best_start = cur;
                    if best_unique >= 6 {
                        break;
                    }
                }
                cur += step;
            }
            best_start
        };

        (
            fps,
            hashes[chosen_start..chosen_start + sample_len].to_vec(),
        )
    } else if media_path.exists() {
        let meta = probe_video(media_path).unwrap_or(dbsrt_core::video::VideoMetadata {
            duration: 0.0,
            fps: 25.0,
        });
        let nb_frames = ((nb_seconds * meta.fps).round() as usize).max(1);
        let start_sec = (meta.duration / 2.0).floor().max(0.0);
        let mut sample = extract_frames_phash(media_path, start_sec, nb_frames).unwrap_or_default();
        let non_blank_unique = sample
            .iter()
            .copied()
            .filter(|&h| !dbsrt_core::is_blank_phash(h))
            .collect::<HashSet<_>>()
            .len();

        if non_blank_unique < 2 && meta.duration > 6.0 {
            for offset_ratio in [0.25, 0.75, 0.1, 0.9] {
                let alt_sec = (meta.duration * offset_ratio).floor().max(0.0);
                if let Ok(alt_sample) = extract_frames_phash(media_path, alt_sec, nb_frames) {
                    let alt_unique = alt_sample
                        .iter()
                        .copied()
                        .filter(|&h| !dbsrt_core::is_blank_phash(h))
                        .collect::<HashSet<_>>()
                        .len();
                    if alt_unique > non_blank_unique {
                        sample = alt_sample;
                        if alt_unique >= 2 {
                            break;
                        }
                    }
                }
            }
        }
        (meta.fps, sample)
    } else {
        (25.0, Vec::new())
    };

    (target_fps, query_sample, existing_phash)
}

pub fn calculate_shift_consensus(
    target_cuts: &[CutItem],
    ref_cuts: &[CutItem],
    search_distance: u32,
    fallback_confidence: f64,
) -> (i64, f64) {
    if ref_cuts.is_empty() || target_cuts.is_empty() {
        return (0, fallback_confidence);
    }

    let mut target_tree = BkTree::new();
    for cut in target_cuts {
        target_tree.insert(BkItem {
            phash: cut.phash,
            offset: cut.offset,
        });
    }

    let mut diff_counter: HashMap<i64, usize> = HashMap::new();
    for ref_cut in ref_cuts {
        let matches = target_tree.search(ref_cut.phash, search_distance);
        for m in matches {
            let diff = m.offset as i64 - ref_cut.offset as i64;
            *diff_counter.entry(diff).or_default() += 1;
        }
    }

    let mut sorted_diffs: Vec<(i64, usize)> = diff_counter.into_iter().collect();
    sorted_diffs.sort_by_key(|a| std::cmp::Reverse(a.1));

    if let Some(&(shift_frames, count)) = sorted_diffs.first() {
        let second_count = sorted_diffs.get(1).map(|(_, c)| *c).unwrap_or(0);
        let dominance = if count > 0 {
            1.0 - (second_count as f64 / count as f64)
        } else {
            0.0
        };
        let support = (count as f64 / ref_cuts.len() as f64).min(1.0);
        let confidence_percent = (dominance * support * PERCENT_SCALE).clamp(0.0, PERCENT_SCALE);
        (shift_frames, confidence_percent)
    } else {
        (0, fallback_confidence)
    }
}

pub fn calculate_sequence_shift_argmin(
    target_hashes: &[u64],
    ref_hashes: &[u64],
    target_fps: f64,
) -> (i64, f64) {
    if target_hashes.is_empty() || ref_hashes.is_empty() {
        return (0, 0.0);
    }

    let fps = if target_fps > 0.0 {
        target_fps
    } else {
        DEFAULT_BASELINE_FPS
    };
    let sample_len = target_hashes.len().min(MAX_CORRELATION_SAMPLE_FRAMES);
    let query = &target_hashes[..sample_len];

    if target_hashes.len() <= ref_hashes.len() {
        let max_pos = ref_hashes.len() - sample_len;
        let step = (fps.round() as usize).max(1);
        let mut coarse_best = 0;
        let mut coarse_min = u64::MAX;
        let mut pos = 0;

        while pos <= max_pos {
            let mut dist_sum = 0u64;
            for j in (0..sample_len).step_by(step) {
                dist_sum += (query[j] ^ ref_hashes[pos + j]).count_ones() as u64;
            }
            if dist_sum < coarse_min {
                coarse_min = dist_sum;
                coarse_best = pos;
            }
            pos += step;
        }

        let refine_radius = step * REFINEMENT_STEP_RADIUS;
        let refine_start = coarse_best.saturating_sub(refine_radius);
        let refine_end = (coarse_best + refine_radius).min(max_pos);
        let mut min_dist_sum = u64::MAX;
        let mut best_offset = coarse_best;

        for r_pos in refine_start..=refine_end {
            let dist_sum: u64 = query
                .iter()
                .zip(&ref_hashes[r_pos..r_pos + sample_len])
                .map(|(&q, &r)| (q ^ r).count_ones() as u64)
                .sum();
            if dist_sum < min_dist_sum {
                min_dist_sum = dist_sum;
                best_offset = r_pos;
            }
        }

        let avg_dist = min_dist_sum as f64 / sample_len as f64;
        let conf = ((1.0 - (avg_dist / PHASH_BIT_DEPTH)) * PERCENT_SCALE).clamp(0.0, PERCENT_SCALE);
        (-(best_offset as i64), conf)
    } else {
        let max_pos = target_hashes.len() - sample_len;
        let step = (fps.round() as usize).max(1);
        let ref_sample_len = ref_hashes.len().min(MAX_CORRELATION_SAMPLE_FRAMES);
        let ref_query = &ref_hashes[..ref_sample_len];

        let mut coarse_best = 0;
        let mut coarse_min = u64::MAX;
        let mut pos = 0;

        while pos <= max_pos {
            let mut dist_sum = 0u64;
            for j in (0..ref_sample_len).step_by(step) {
                dist_sum += (ref_query[j] ^ target_hashes[pos + j]).count_ones() as u64;
            }
            if dist_sum < coarse_min {
                coarse_min = dist_sum;
                coarse_best = pos;
            }
            pos += step;
        }

        let refine_radius = step * REFINEMENT_STEP_RADIUS;
        let refine_start = coarse_best.saturating_sub(refine_radius);
        let refine_end = (coarse_best + refine_radius).min(max_pos);
        let mut min_dist_sum = u64::MAX;
        let mut best_offset = coarse_best;

        for t_pos in refine_start..=refine_end {
            let dist_sum: u64 = ref_query
                .iter()
                .zip(&target_hashes[t_pos..t_pos + ref_sample_len])
                .map(|(&r, &t)| (r ^ t).count_ones() as u64)
                .sum();
            if dist_sum < min_dist_sum {
                min_dist_sum = dist_sum;
                best_offset = t_pos;
            }
        }

        let avg_dist = min_dist_sum as f64 / ref_sample_len as f64;
        let conf = ((1.0 - (avg_dist / PHASH_BIT_DEPTH)) * PERCENT_SCALE).clamp(0.0, PERCENT_SCALE);
        (best_offset as i64, conf)
    }
}

pub fn compute_media_phash_shift(
    media_path: &Path,
    phash_dir: &Path,
    matched_stem: &str,
    match_conf: f64,
    target_fps: f64,
    existing_phash: &Option<ExistingPhashData>,
    search_distance: u32,
) -> (i64, f64, f64) {
    compute_media_phash_shift_multi(
        media_path,
        &[phash_dir.to_path_buf()],
        matched_stem,
        match_conf,
        target_fps,
        existing_phash,
        search_distance,
    )
}

pub fn compute_media_phash_shift_multi(
    media_path: &Path,
    phash_dirs: &[PathBuf],
    matched_stem: &str,
    match_conf: f64,
    target_fps: f64,
    existing_phash: &Option<ExistingPhashData>,
    search_distance: u32,
) -> (i64, f64, f64) {
    let ref_hashes_opt = phash_dirs
        .iter()
        .find_map(|dir| read_ref_phash_for_media(dir, matched_stem).ok());

    let (shift_frames, confidence_percent) = match ref_hashes_opt {
        Some(ref_hashes) => {
            let target_hashes = if let Some((_, ref hashes, _)) = existing_phash {
                hashes.clone()
            } else if let Ok(m) = probe_video(media_path) {
                let total_frames = (m.duration * m.fps).floor() as usize;
                extract_frames_phash(media_path, 0.0, total_frames).unwrap_or_default()
            } else {
                Vec::new()
            };

            if !target_hashes.is_empty() {
                let target_cuts = detect_cuts(&target_hashes, DEFAULT_CUT_THRESHOLD);
                let ref_cuts = detect_cuts(&ref_hashes, DEFAULT_CUT_THRESHOLD);
                let (sf_cut, conf_cut) = if target_cuts.len() >= MIN_CUTS_FOR_CONSENSUS
                    && ref_cuts.len() >= MIN_CUTS_FOR_CONSENSUS
                {
                    calculate_shift_consensus(&target_cuts, &ref_cuts, search_distance, match_conf)
                } else {
                    (0, 0.0)
                };

                if conf_cut >= MIN_CUT_CONSENSUS_CONFIDENCE {
                    (sf_cut, conf_cut)
                } else {
                    calculate_sequence_shift_argmin(&target_hashes, &ref_hashes, target_fps)
                }
            } else {
                (0, match_conf)
            }
        }
        None => (0, match_conf),
    };

    let fps = if target_fps > 0.0 {
        target_fps
    } else {
        DEFAULT_FALLBACK_FPS
    };
    let shift_ms = (shift_frames as f64 / fps) * MILLIS_PER_SEC;
    (shift_frames, shift_ms, confidence_percent)
}
