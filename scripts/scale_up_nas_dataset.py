#!/usr/bin/env python3
"""Massive NAS Dataset Scale-Up & Ground-Truth Certification Pipeline.

Discovers a stratified, diverse corpus of 108+ unindexed videos on the NAS
(spanning 35+ TV series, anime, shorts, and 25+ feature films across 720p,
1080p, 2160p, x264, x265/HEVC, and multi-release pairs), extracts their full
64-bit 2D DCT-II pHash streams (`dbsrt export-phash`), imports millions of
frame hashes into PostgreSQL 17 (`bktree_ops`), extracts/associates external
or embedded MKV subtitles (`fr`/`en`), certifies ground-truth
`(media <-> frames <-> subtitles <-> sub_frames)` couplings, and packs a
unified `.dbphash` memory-mapped index.
"""

from __future__ import annotations

import argparse
import getpass
import json
import os
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path


def default_nas_root() -> Path:
    env_root = os.environ.get("NAS_ROOT")
    if env_root:
        return Path(env_root)
    return Path("/media") / getpass.getuser() / "Synology"


def detect_srt_lang(filename: str) -> str | None:
    lower = filename.lower()
    if lower.endswith(".srt"):
        lower = lower[:-4]
    tokens = lower.replace(".", "_").replace("-", "_").split("_")
    skip = {
        "forced",
        "sdh",
        "hi",
        "shift",
        "resync",
        "synced",
        "track",
        "sub",
        "subs",
    }
    for tok in reversed(tokens):
        if tok.isdigit() or tok in skip:
            continue
        if tok in {"fr", "fre", "fra", "french", "vff", "vf", "vostfr"}:
            return "fr"
        if tok in {"en", "eng", "english"}:
            return "en"
        break
    return None


def extract_embedded_srts_for_video(video_path: Path, out_srt_dir: Path) -> list[tuple[str, Path]]:
    """Extracts embedded FR and EN text subtitle streams (subrip/ass/webvtt) from MKV/MP4 into out_srt_dir."""
    probe_cmd = [
        "ffprobe",
        "-v",
        "error",
        "-select_streams",
        "s",
        "-show_entries",
        "stream=index,codec_name:stream_tags=language,title",
        "-of",
        "json",
        str(video_path),
    ]
    res = subprocess.run(probe_cmd, capture_output=True, text=True)
    if res.returncode != 0 or not res.stdout.strip():
        return []
    try:
        data = json.loads(res.stdout)
    except json.JSONDecodeError:
        return []

    streams = data.get("streams", [])
    text_codecs = {"subrip", "srt", "ass", "ssa", "mov_text", "webvtt", "text"}
    chosen_by_lang: dict[str, tuple[int, bool]] = {}

    for st in streams:
        codec = st.get("codec_name", "").lower()
        if codec not in text_codecs:
            continue
        idx = st.get("index")
        if idx is None:
            continue
        tags = st.get("tags") or {}
        raw_lang = (tags.get("language") or tags.get("LANGUAGE") or "").lower()
        title = (tags.get("title") or tags.get("TITLE") or "").lower()
        is_forced = "forced" in title or "sign" in title

        lang = None
        if raw_lang in {"fre", "fra", "fr"} or "fran" in title or "french" in title:
            lang = "fr"
        elif raw_lang in {"eng", "en"} or "english" in title:
            lang = "en"

        if not lang:
            continue

        # Prefer non-forced full dialogue track over forced track
        if lang not in chosen_by_lang or (chosen_by_lang[lang][1] and not is_forced):
            chosen_by_lang[lang] = (int(idx), is_forced)

    extracted: list[tuple[str, Path]] = []
    for lang, (stream_idx, _is_forced) in chosen_by_lang.items():
        dest = out_srt_dir / f"{video_path.stem}.{lang}.srt"
        if dest.exists() and dest.stat().st_size >= 1024:
            extracted.append((lang, dest))
            continue
        ext_cmd = [
            "ffmpeg",
            "-y",
            "-v",
            "error",
            "-i",
            str(video_path),
            "-map",
            f"0:{stream_idx}",
            "-c:s",
            "srt",
            str(dest),
        ]
        r_ext = subprocess.run(ext_cmd, capture_output=True, text=True)
        if r_ext.returncode == 0 and dest.exists() and dest.stat().st_size >= 1024:
            extracted.append((lang, dest))
        elif dest.exists() and dest.stat().st_size < 1024:
            try:
                dest.unlink()
            except OSError:
                pass

    return extracted


def build_nas_catalog(nas_root: Path) -> tuple[dict[str, list[Path]], list[tuple[Path, list[Path], str]]]:
    """Single-pass walk of NAS to map SRT files and discover all video candidates."""
    tv_srt_dir = nas_root / "tvshow" / "SRT"
    srt_by_stem: dict[str, list[Path]] = {}

    if tv_srt_dir.is_dir():
        for entry in os.scandir(tv_srt_dir):
            if not entry.name.lower().endswith(".srt"):
                continue
            try:
                if entry.stat().st_size < 1024:
                    continue
            except OSError:
                continue
            base = entry.name
            for sep in (".mkv_", ".mp4_", ".avi_", ".webm_"):
                if sep in base:
                    base = base.split(sep)[0]
                    break
            else:
                if base.lower().endswith(".srt"):
                    base = base[:-4]
                    for l_ext in (".fr", ".en", ".fre", ".eng", ".french", ".english"):
                        if base.lower().endswith(l_ext):
                            base = base[: -len(l_ext)]
                            break
            srt_by_stem.setdefault(base, []).append(Path(entry.path))

    videos: list[tuple[Path, list[Path], str]] = []
    video_exts = (".mkv", ".mp4", ".avi", ".webm")

    for root, _dirs, files in os.walk(nas_root):
        if Path(root) == tv_srt_dir:
            continue
        local_srts = [Path(root) / f for f in files if f.lower().endswith(".srt")]
        valid_local_srts = []
        for s in local_srts:
            try:
                if s.stat().st_size >= 1024:
                    valid_local_srts.append(s)
            except OSError:
                pass

        for f in files:
            if not f.lower().endswith(video_exts):
                continue
            vpath = Path(root) / f
            stem = vpath.stem
            matched_srts: list[Path] = []
            if stem in srt_by_stem:
                matched_srts.extend(srt_by_stem[stem])
            for ls in valid_local_srts:
                if ls.name.lower().startswith(stem.lower()[:20]) or len(valid_local_srts) == 1:
                    if ls not in matched_srts:
                        matched_srts.append(ls)

            rel = str(vpath.relative_to(nas_root))
            if rel.startswith("tvshow/"):
                parts = rel.split("/")
                category = f"tv:{parts[1]}" if len(parts) > 1 else "tv:misc"
            else:
                category = "movie"
            videos.append((vpath, matched_srts, category))

    return srt_by_stem, videos


def select_stratified_videos(
    videos: list[tuple[Path, list[Path], str]],
    existing_phash_stems: set[str],
    target_count: int,
) -> list[tuple[Path, list[Path], str]]:
    """Selects a diverse set of unindexed videos across shows, movies, codecs, and multi-encode pairs."""
    unindexed = [(v, srts, cat) for (v, srts, cat) in videos if v.stem not in existing_phash_stems]

    selected: list[tuple[Path, list[Path], str]] = []
    selected_paths: set[Path] = set()

    def add_item(item: tuple[Path, list[Path], str]) -> bool:
        v, _srts, _cat = item
        if v in selected_paths:
            return False
        try:
            size_mb = v.stat().st_size / (1024 * 1024)
        except OSError:
            return False
        if size_mb < 15 or size_mb > 3500:
            return False
        selected.append(item)
        selected_paths.add(v)
        return True

    by_category: dict[str, list[tuple[Path, list[Path], str]]] = {}
    for item in unindexed:
        by_category.setdefault(item[2], []).append(item)

    for _cat, items in by_category.items():
        items.sort(
            key=lambda x: (
                0 if x[1] else 1,
                x[0].stat().st_size if x[0].exists() else 999_999_999_999,
            )
        )

    tv_cats = sorted([c for c in by_category if c.startswith("tv:")])
    for round_idx in range(3):
        for cat in tv_cats:
            items = by_category[cat]
            if round_idx < len(items):
                add_item(items[round_idx])
                if len(selected) >= int(target_count * 0.72):
                    break
        if len(selected) >= int(target_count * 0.72):
            break

    movie_items = by_category.get("movie", [])
    for item in movie_items:
        if any(k in item[0].name.lower() for k in ("blue.bayou", "cmon.cmon", "cosmic.sin", "apex.2021")):
            if item[0] not in selected_paths:
                selected.append(item)
                selected_paths.add(item[0])

    for item in movie_items:
        if len(selected) >= target_count:
            break
        add_item(item)

    if len(selected) < target_count:
        for item in unindexed:
            if len(selected) >= target_count:
                break
            if item[1]:
                add_item(item)

    return selected[:target_count]


def certify_existing_phash_couplings(
    dbsrt_bin: Path,
    phash_dir: Path,
    local_srt_dir: Path,
    srt_by_stem: dict[str, list[Path]],
    db_host: str,
    db_port: int,
) -> tuple[int, int]:
    """Certifies all matching (phash <-> srt) pairs for already-extracted .phash files into PostgreSQL."""
    certified_count = 0
    skipped_or_oob = 0
    phash_files = sorted(phash_dir.glob("*.phash"))

    for pf in phash_files:
        stem = pf.stem
        candidate_srts: list[Path] = []
        if stem in srt_by_stem:
            candidate_srts.extend(srt_by_stem[stem])
        for local_s in local_srt_dir.glob(f"{stem}*.srt"):
            if local_s not in candidate_srts:
                candidate_srts.append(local_s)

        if not candidate_srts:
            continue

        best_by_lang: dict[str, Path] = {}
        for s in candidate_srts:
            lang = detect_srt_lang(s.name) or "en"
            try:
                sz = s.stat().st_size
            except OSError:
                continue
            if sz < 1024:
                continue
            if lang not in best_by_lang or sz > best_by_lang[lang].stat().st_size:
                best_by_lang[lang] = s

        for lang, srt_path in best_by_lang.items():
            cmd = [
                str(dbsrt_bin),
                "certify-coupling",
                "--db-only",
                "--phash-file",
                str(pf),
                "--subtitle",
                str(srt_path),
                "--lang",
                lang,
                "--db-host",
                db_host,
                "--db-port",
                str(db_port),
            ]
            res = subprocess.run(cmd, capture_output=True, text=True)
            if res.returncode == 0:
                certified_count += 1
            else:
                skipped_or_oob += 1

    return certified_count, skipped_or_oob


def process_one_video(
    idx: int,
    total: int,
    video_path: Path,
    srts: list[Path],
    category: str,
    dbsrt_bin: Path,
    phash_dir: Path,
    local_srt_dir: Path,
    db_host: str,
    db_port: int,
) -> dict:
    t0 = time.monotonic()
    out_phash = phash_dir / f"{video_path.stem}.phash"

    if not out_phash.exists() or out_phash.stat().st_size < 100:
        cmd_export = [
            str(dbsrt_bin),
            "export-phash",
            "--media",
            str(video_path),
            "--output",
            str(out_phash),
            "--hwaccel",
            "disabled",
        ]
        res_exp = subprocess.run(cmd_export, capture_output=True, text=True)
        if res_exp.returncode != 0 or not out_phash.exists():
            return {
                "index": idx,
                "video": video_path.name,
                "category": category,
                "success": False,
                "error": res_exp.stderr[-300:] if res_exp.stderr else "export failed",
                "elapsed_s": round(time.monotonic() - t0, 2),
            }

    frames_count = max(0, (out_phash.stat().st_size - 4) // 8)

    # Import into PostgreSQL 17
    cmd_imp = [
        str(dbsrt_bin),
        "import-phash",
        "--file",
        str(out_phash),
        "--db-host",
        db_host,
        "--db-port",
        str(db_port),
    ]
    subprocess.run(cmd_imp, capture_output=True, text=True)

    # Gather external SRTs + fallback to embedded MKV SRTs if FR or EN missing
    best_by_lang: dict[str, Path] = {}
    for s in srts:
        lang = detect_srt_lang(s.name) or "en"
        try:
            sz = s.stat().st_size
        except OSError:
            continue
        if sz < 1024:
            continue
        if lang not in best_by_lang or sz > best_by_lang[lang].stat().st_size:
            best_by_lang[lang] = s

    if not best_by_lang:
        for lang, emb_path in extract_embedded_srts_for_video(video_path, local_srt_dir):
            if lang not in best_by_lang:
                best_by_lang[lang] = emb_path

    certified_langs = []
    for lang, srt_path in best_by_lang.items():
        cmd_cert = [
            str(dbsrt_bin),
            "certify-coupling",
            "--db-only",
            "--phash-file",
            str(out_phash),
            "--subtitle",
            str(srt_path),
            "--lang",
            lang,
            "--db-host",
            db_host,
            "--db-port",
            str(db_port),
        ]
        res_c = subprocess.run(cmd_cert, capture_output=True, text=True)
        if res_c.returncode == 0:
            certified_langs.append(lang)

    elapsed = round(time.monotonic() - t0, 2)
    fps_proc = round(frames_count / max(elapsed, 0.01), 1)
    print(
        f"[{idx:>3}/{total}] OK {category:<28} {video_path.name[:52]:<52} -> {frames_count:>7} frames in {elapsed:>5.1f}s ({fps_proc:>6.0f} fps) certs={certified_langs}",
        flush=True,
    )
    return {
        "index": idx,
        "video": video_path.name,
        "category": category,
        "success": True,
        "frames": frames_count,
        "certified_langs": certified_langs,
        "elapsed_s": elapsed,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Scale up pydbsrt pHash & PostgreSQL dataset from NAS.")
    parser.add_argument("--nas-root", type=Path, default=default_nas_root())
    parser.add_argument("--phash-dir", type=Path, default=Path("data/__PHASH__"))
    parser.add_argument("--subtitles-dir", type=Path, default=Path("data/SRT"))
    parser.add_argument("--dbsrt-bin", type=Path, default=Path("./target/release/dbsrt"))
    parser.add_argument("--target-new-videos", type=int, default=108)
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--db-host", type=str, default=os.environ.get("PSQL_IP", "127.0.0.1"))
    parser.add_argument("--db-port", type=int, default=int(os.environ.get("PSQL_PORT", "5433")))
    parser.add_argument("--summary-out", type=Path, default=Path("/tmp/nas_scale_up_summary.json"))
    args = parser.parse_args()

    if not args.dbsrt_bin.exists():
        print(f"Error: {args.dbsrt_bin} not found.", file=sys.stderr)
        return 1

    args.phash_dir.mkdir(parents=True, exist_ok=True)
    args.subtitles_dir.mkdir(parents=True, exist_ok=True)
    t_start = time.monotonic()

    print(f"=== STEP 1: Scanning NAS catalog at {args.nas_root} ===", flush=True)
    srt_by_stem, all_videos = build_nas_catalog(args.nas_root)
    existing_stems = {p.stem for p in args.phash_dir.glob("*.phash")}
    print(
        f"Found {len(all_videos)} videos on NAS, {len(srt_by_stem)} SRT release stems in tvshow/SRT, {len(existing_stems)} existing .phash files.",
        flush=True,
    )

    print("=== STEP 2: Certifying existing .phash <-> NAS .srt couplings in PostgreSQL 17 ===", flush=True)
    cert_existing, oob_existing = certify_existing_phash_couplings(
        args.dbsrt_bin,
        args.phash_dir,
        args.subtitles_dir,
        srt_by_stem,
        args.db_host,
        args.db_port,
    )
    print(
        f"Certified {cert_existing} subtitle tracks on existing .phash medias (skipped/OOB={oob_existing}).",
        flush=True,
    )

    # Use initial existing count (126) so any videos already extracted in interrupted runs are still included in the 108 batch
    baseline_stems = {p.stem for p in args.phash_dir.glob("*.phash") if p.stat().st_mtime < (time.time() - 10800)}
    selected = select_stratified_videos(all_videos, baseline_stems, args.target_new_videos)
    categories_covered = sorted({cat for (_, _, cat) in selected})
    print(
        f"=== STEP 3: Extracting pHash + Importing + Certifying on {len(selected)} NEW diverse NAS videos across {len(categories_covered)} categories (workers={args.workers}) ===",
        flush=True,
    )

    results = []
    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        futures = [
            pool.submit(
                process_one_video,
                i + 1,
                len(selected),
                vpath,
                srts,
                cat,
                args.dbsrt_bin,
                args.phash_dir,
                args.subtitles_dir,
                args.db_host,
                args.db_port,
            )
            for i, (vpath, srts, cat) in enumerate(selected)
        ]
        for fut in as_completed(futures):
            results.append(fut.result())

    results.sort(key=lambda r: r["index"])
    succeeded = [r for r in results if r.get("success")]
    new_frames = sum(r.get("frames", 0) for r in succeeded)
    new_certs = sum(len(r.get("certified_langs", [])) for r in succeeded)

    print("=== STEP 4: Packing unified .dbphash index across all .phash files ===", flush=True)
    dbphash_out = Path("data/catalog.dbphash")
    subprocess.run(
        [
            str(args.dbsrt_bin),
            "pack",
            "--phash-dir",
            str(args.phash_dir),
            "--output",
            str(dbphash_out),
        ],
        capture_output=True,
        text=True,
    )

    total_phash_files = len(list(args.phash_dir.glob("*.phash")))
    total_elapsed = round(time.monotonic() - t_start, 2)

    summary = {
        "total_elapsed_s": total_elapsed,
        "initial_phash_files": len(baseline_stems),
        "new_videos_targeted": len(selected),
        "new_videos_extracted": len(succeeded),
        "new_frames_extracted": new_frames,
        "total_phash_files_now": total_phash_files,
        "distinct_categories_in_new_batch": len(categories_covered),
        "categories": categories_covered,
        "certified_existing_subtitles": cert_existing,
        "certified_new_subtitles": new_certs,
        "dbphash_path": str(dbphash_out),
        "results": results,
    }
    args.summary_out.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(
        f"=== SCALE-UP COMPLETE in {total_elapsed:.1f}s: {len(succeeded)}/{len(selected)} new videos ({new_frames:,} new frames), total {total_phash_files} .phash files ===",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
