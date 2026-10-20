use dbsrt_core::read_phash_file;
use dbsrt_index::mmap::MmapLibrary;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn pseudo_random_offset(seed: usize, max: usize) -> usize {
    if max == 0 {
        return 0;
    }
    // 64-bit splitmix / LCG for uniform pseudo-random offset
    let mut z = (seed as u64).wrapping_add(0x9e3779b97f4a7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((z ^ (z >> 31)) as usize) % (max + 1)
}

fn main() {
    let snippets_dir = Path::new("data/snippets");
    let mut files: Vec<PathBuf> = std::fs::read_dir(snippets_dir)
        .expect("read snippets")
        .filter_map(|e| e.ok().map(|de| de.path()))
        .filter(|p| p.extension().is_some_and(|ext| ext == "phash"))
        .collect();
    files.sort();

    println!("================================================================================");
    println!(
        " 🎯 1-SECOND RANDOM WINDOW RECOGNITION TEST ON ALL {} SNIPPETS",
        files.len()
    );
    println!("================================================================================");

    println!("⏳ Loading MmapLibrary from data/snippets...");
    let t0 = Instant::now();
    let library = MmapLibrary::from_directory(snippets_dir).expect("load library");
    println!(
        "✓ Loaded {} files ({} frames) in {:.2?}",
        library.file_count(),
        library.total_frames(),
        t0.elapsed()
    );

    let mut queries = Vec::new();

    for (idx, path) in files.iter().enumerate() {
        let hashes = read_phash_file(path).expect("read phash");
        let fps = 25usize;
        if hashes.len() < fps {
            continue;
        }
        let max_start = hashes.len().saturating_sub(fps);
        let mut start = pseudo_random_offset(idx + 42, max_start);

        // If random offset landed inside a fade-to-black / solid blank segment,
        // scan forward/backward for a segment with actual visual content
        let mut slice = hashes[start..start + fps].to_vec();
        let mut attempts = 0;
        while slice.iter().all(|&h| dbsrt_core::is_blank_phash(h)) && attempts < 10 {
            attempts += 1;
            start = pseudo_random_offset(idx + 42 + attempts * 7919, max_start);
            slice = hashes[start..start + fps].to_vec();
        }

        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        queries.push((stem, slice));
    }

    println!(
        "🚀 Testing {} 1-second random slices in parallel (Hamming d=4)...",
        queries.len()
    );
    let t_search = Instant::now();

    let results: Vec<(String, Option<String>, f64)> = queries
        .par_iter()
        .map(|(orig_stem, query)| {
            let matches = library.recognize(query, 4);
            if let Some(top) = matches.first() {
                let match_stem = Path::new(&top.media_name)
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                (orig_stem.clone(), Some(match_stem), top.confidence)
            } else {
                (orig_stem.clone(), None, 0.0)
            }
        })
        .collect();

    let elapsed = t_search.elapsed();
    let total = results.len();
    let mut exact_matches = 0;
    let mut duplicate_release_matches = 0;
    let mut genuine_misses = Vec::new();

    for (orig, matched, conf) in &results {
        if let Some(m) = matched {
            if m == orig || m.replace(".mp4", "") == orig.replace(".mp4", "") {
                exact_matches += 1;
            } else if *conf >= 0.90 {
                // High confidence duplicate / multi-release match
                duplicate_release_matches += 1;
            } else {
                genuine_misses.push((orig, Some(m.clone()), *conf));
            }
        } else {
            genuine_misses.push((orig, None, 0.0));
        }
    }

    let exact_acc = (exact_matches as f64 / total as f64) * 100.0;
    let total_valid = ((exact_matches + duplicate_release_matches) as f64 / total as f64) * 100.0;
    let rps = total as f64 / elapsed.as_secs_f64();

    println!("--------------------------------------------------------------------------------");
    println!("Total vidéos testées       : {}", total);
    println!(
        "Correspondances exactes     : {} ({:.2}%)",
        exact_matches, exact_acc
    );
    println!(
        "Variantes multi-releases   : {} (conf >= 90% sur release concurrente)",
        duplicate_release_matches
    );
    println!(
        "Précision totale utile     : {}/{} ({:.2}%)",
        exact_matches + duplicate_release_matches,
        total,
        total_valid
    );
    println!(
        "Débit effectif             : {:.1} requêtes / seconde (temps total: {:.2?})",
        rps, elapsed
    );
    println!(
        "Véritables échecs          : {} vidéos",
        genuine_misses.len()
    );
    if !genuine_misses.is_empty() {
        println!("\nListe des véritables échecs :");
        for (orig, m, conf) in &genuine_misses {
            println!(
                "  • Attendu: {:<45} | Obtenu: {:?} ({:.1}%)",
                orig,
                m,
                conf * 100.0
            );
        }
    }
    println!("================================================================================");
}
