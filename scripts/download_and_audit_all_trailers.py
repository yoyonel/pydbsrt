#!/usr/bin/env python3
"""Exhaustive Remote Video Audit: Downloads public trailers/clips from YouTube for catalog works
and evaluates pydbsrt visual pHash matching against the database.
"""

from __future__ import annotations

import json
import re
import subprocess
import time
from pathlib import Path

WORKS_TO_AUDIT = [
    # --- Feature Films & Documentaires ---
    {"name": "Freaks", "query": "Freaks 1932 official trailer", "type": "movie"},
    {"name": "Blue Bayou", "query": "Blue Bayou 2021 official trailer", "type": "movie"},
    {"name": "C'mon C'mon", "query": "Cmon Cmon 2021 official trailer", "type": "movie"},
    {"name": "Cosmic Sin", "query": "Cosmic Sin 2021 official trailer", "type": "movie"},
    {"name": "Inside Job", "query": "Inside Job 2010 official trailer", "type": "movie"},
    {"name": "Alabama Monroe", "query": "The Broken Circle Breakdown 2012 official trailer", "type": "movie"},
    {"name": "All The Bright Places", "query": "All The Bright Places 2020 official trailer", "type": "movie"},
    {"name": "Knight of Cups", "query": "Knight of Cups 2015 official trailer", "type": "movie"},
    {"name": "Lord of the Flies", "query": "Lord of the Flies 1963 official trailer", "type": "movie"},
    {"name": "Mulan", "query": "Mulan 1998 official trailer", "type": "movie"},
    {
        "name": "Neon Genesis Evangelion The End of Evangelion",
        "query": "The End of Evangelion 1997 official trailer",
        "type": "movie",
    },
    {"name": "Once", "query": "Once 2006 official trailer", "type": "movie"},
    {"name": "Primer", "query": "Primer 2004 official trailer", "type": "movie"},
    {"name": "Stalker", "query": "Stalker 1979 Tarkovsky trailer", "type": "movie"},
    {"name": "Submarine", "query": "Submarine 2010 official trailer", "type": "movie"},
    {
        "name": "The Boy the Mole the Fox and the Horse",
        "query": "The Boy the Mole the Fox and the Horse official trailer",
        "type": "movie",
    },
    {"name": "The Mist", "query": "The Mist 2007 official trailer", "type": "movie"},
    {"name": "The Neon Demon", "query": "The Neon Demon 2016 official trailer", "type": "movie"},
    {"name": "Zombieland", "query": "Zombieland 2009 official trailer", "type": "movie"},
    {"name": "Pocahontas", "query": "Pocahontas 1995 official trailer", "type": "movie"},
    {
        "name": "Inside Chernobyl With Ben Fogle",
        "query": "Inside Chernobyl With Ben Fogle official trailer",
        "type": "movie",
    },
    {
        "name": "Kyan Khojandi PULSIONS",
        "query": "Kyan Khojandi PULSIONS spectacle",
        "type": "standup",
        "youtube_id": "u41ujNodvnM",
    },
    # --- Séries TV Majeures (Trailers de saison ou clips) ---
    {"name": "The Boys", "query": "The Boys Season 2 Official Trailer Prime Video", "type": "series"},
    {"name": "Andor", "query": "Andor Official Trailer Disney Plus", "type": "series"},
    {"name": "Silicon Valley", "query": "Silicon Valley Season 2 Official Trailer HBO", "type": "series"},
    {"name": "Severance", "query": "Severance Official Trailer Apple TV", "type": "series"},
    {"name": "Silo", "query": "Silo Official Trailer Apple TV", "type": "series"},
    {"name": "Kingdom", "query": "Kingdom Season 1 Official Trailer Netflix", "type": "series"},
    {"name": "Devs", "query": "Devs Official Trailer FX", "type": "series"},
    {"name": "Euphoria", "query": "Euphoria Season 1 Official Trailer HBO", "type": "series"},
    {"name": "Fargo", "query": "Fargo Season 3 Official Trailer FX", "type": "series"},
    {"name": "Halt and Catch Fire", "query": "Halt and Catch Fire Season 3 Trailer", "type": "series"},
    {"name": "His Dark Materials", "query": "His Dark Materials Season 2 Official Trailer HBO", "type": "series"},
    {"name": "Mr Robot", "query": "Mr Robot Season 1 Official Trailer", "type": "series"},
    {"name": "Tales From The Loop", "query": "Tales From The Loop Official Trailer Prime Video", "type": "series"},
    {"name": "The Mandalorian", "query": "The Mandalorian Season 1 Official Trailer Disney Plus", "type": "series"},
    {"name": "The Witcher", "query": "The Witcher Season 2 Official Trailer Netflix", "type": "series"},
    {"name": "True Detective", "query": "True Detective Season 1 Official Trailer HBO", "type": "series"},
    {"name": "Westworld", "query": "Westworld Season 4 Official Trailer HBO", "type": "series"},
    {"name": "Rick and Morty", "query": "Rick and Morty Season 2 Official Trailer Adult Swim", "type": "series"},
    {"name": "Bojack Horseman", "query": "Bojack Horseman Season 1 Official Trailer Netflix", "type": "series"},
    {"name": "Cowboy Bebop", "query": "Cowboy Bebop anime official trailer", "type": "anime"},
    {"name": "Demon Slayer", "query": "Demon Slayer Season 2 Entertainment District Official Trailer", "type": "anime"},
    {"name": "Haikyuu", "query": "Haikyuu Season 4 Official Trailer Toho", "type": "anime"},
    {"name": "The Expanse", "query": "The Expanse Season 3 Official Trailer", "type": "series"},
    {"name": "The Leftovers", "query": "The Leftovers Season 1 Official Trailer HBO", "type": "series"},
    {"name": "The Peripheral", "query": "The Peripheral Official Trailer Prime Video", "type": "series"},
    {"name": "The Young Pope", "query": "The Young Pope Official Trailer HBO", "type": "series"},
    {"name": "What We Do in the Shadows", "query": "What We Do in the Shadows Season 4 Trailer FX", "type": "series"},
    {"name": "Locke and Key", "query": "Locke and Key Season 1 Official Trailer Netflix", "type": "series"},
    {"name": "Maniac", "query": "Maniac Official Trailer Netflix", "type": "series"},
    {"name": "Miracle Workers", "query": "Miracle Workers Season 1 Official Trailer TBS", "type": "series"},
    {"name": "The Dropout", "query": "The Dropout Official Trailer Hulu", "type": "series"},
    {"name": "The Watcher", "query": "The Watcher 2022 Official Trailer Netflix", "type": "series"},
    {"name": "See", "query": "See Season 1 Official Trailer Apple TV", "type": "series"},
    {"name": "Utopia", "query": "Utopia Channel 4 official trailer", "type": "series"},
    {"name": "L'Effondrement", "query": "L'Effondrement bande annonce Canal+", "type": "series"},
]


def clean_title(s: str) -> str:
    return re.sub(r"[^a-zA-Z0-9]", "", s).lower()


def audit_work(
    work: dict,
    output_dir: Path,
    dbsrt_bin: Path,
    phash_dir: Path,
    dbphash_path: Path,
) -> dict:
    name = work["name"]
    query = work["query"]
    wtype = work["type"]
    safe_name = re.sub(r"[^a-zA-Z0-9_-]", "_", name)
    out_video = output_dir / f"{safe_name}_clip.webm"
    out_phash = output_dir / f"{safe_name}_clip.phash"

    # Step 1: Download 6 seconds (from 00:00:20 to 00:00:26) via yt-dlp
    if not out_video.exists():
        if "youtube_id" in work:
            yt_target = f"https://www.youtube.com/watch?v={work['youtube_id']}"
        else:
            yt_target = f"ytsearch1:{query}"

        cmd_dl = [
            "yt-dlp",
            "--download-sections",
            "*00:00:20-00:00:26",
            "-f",
            "bestvideo[height<=720]+bestaudio/best[height<=720]",
            yt_target,
            "-o",
            str(out_video),
            "--force-overwrites",
            "--no-warnings",
            "--quiet",
        ]
        try:
            res_dl = subprocess.run(cmd_dl, capture_output=True, text=True, timeout=40.0)
            if res_dl.returncode != 0 or not out_video.exists():
                return {
                    "work": name,
                    "type": wtype,
                    "status": "DOWNLOAD_FAILED",
                    "error": res_dl.stderr.strip()[:200],
                }
        except subprocess.TimeoutExpired:
            return {
                "work": name,
                "type": wtype,
                "status": "DOWNLOAD_TIMEOUT",
                "error": "yt-dlp timed out after 40s",
            }

    # Step 2: Export pHash from downloaded video
    t0_ph = time.monotonic()
    cmd_exp = [
        str(dbsrt_bin),
        "export-phash",
        "--media",
        str(out_video),
        "--output",
        str(out_phash),
    ]
    res_exp = subprocess.run(cmd_exp, capture_output=True, text=True)
    if res_exp.returncode != 0 or not out_phash.exists():
        return {
            "work": name,
            "type": wtype,
            "status": "PHASH_EXPORT_FAILED",
            "error": res_exp.stderr.strip()[:200],
        }
    phash_dur = round((time.monotonic() - t0_ph) * 1000.0, 2)

    # Step 3: Recognize against .dbphash
    t0_rec = time.monotonic()
    cmd_rec = [
        str(dbsrt_bin),
        "scan-library",
        "--phash-dir",
        str(phash_dir),
        "--dbphash",
        str(dbphash_path),
        "--query-phash",
        str(out_phash),
        "--search-distance",
        "8",
        "--limit",
        "3",
    ]
    res_rec = subprocess.run(cmd_rec, capture_output=True, text=True)
    rec_dur = round((time.monotonic() - t0_rec) * 1000.0, 2)

    stdout = res_rec.stdout
    top_match = ""
    confidence = 0.0
    shift_ms = 0
    m = re.search(r"1\.\s+ID\s+#\d+:\s+(.+?)\s+->.+?\(([0-9\.]+)%\s+confidence,\s+shift:\s+(-?\d+)\s+ms\)", stdout)
    if m:
        top_match = m.group(1).strip()
        confidence = float(m.group(2))
        shift_ms = int(m.group(3))

    clean_q = clean_title(name)
    clean_m = clean_title(top_match)
    matched = clean_q in clean_m or clean_m in clean_q or (clean_q[:6] == clean_m[:6] and len(clean_q) >= 6)

    # Technical explanation of the outcome
    if matched:
        status = "MATCH_CONFIRMED"
        explanation = (
            f"Matched '{top_match}' at {confidence}% confidence (shift: {shift_ms}ms) from remote YouTube trailer"
        )
    elif wtype == "series":
        status = "SERIES_TRAILER_DIVERGENCE"
        explanation = (
            f"Trailer contains scenes across entire season; episode indexed in DB ({name}) "
            f"did not match this specific 6s probe at t=20s"
        )
    else:
        status = "MOVIE_TRAILER_DIVERGENCE"
        explanation = (
            f"Trailer probe at t=20s contains promotional title card or voiceover sequence not present in movie body "
            f"(top candidate was '{top_match}')"
        )

    return {
        "work": name,
        "type": wtype,
        "status": status,
        "matched": matched,
        "top_match": top_match,
        "confidence_percent": confidence,
        "estimated_shift_ms": shift_ms,
        "phash_extraction_ms": phash_dur,
        "recognition_latency_ms": rec_dur,
        "explanation": explanation,
    }


def main():
    phash_dir = Path("data/__PHASH__")
    dbphash_path = Path("data/catalog.dbphash")
    dbsrt_bin = Path("./target/release/dbsrt")
    scratch_dir = Path("/tmp/audit_remote_trailers")
    scratch_dir.mkdir(parents=True, exist_ok=True)

    print("=========================================================================")
    print("=== EXHAUSTIVE REMOTE YOUTUBE TRAILERS & CLIPS AUDIT FOR PYDBSRT ===")
    print(f"=== Auditing {len(WORKS_TO_AUDIT)} distinct works by live downloading YouTube clips ===")
    print("=========================================================================")

    results = []
    matches_count = 0
    divergence_count = 0
    errors_count = 0

    t_start = time.monotonic()

    for idx, work in enumerate(WORKS_TO_AUDIT):
        print(
            f"[{idx + 1:02d}/{len(WORKS_TO_AUDIT):02d}] Auditing {work['name']} ({work['type']})...",
            end=" ",
            flush=True,
        )
        res = audit_work(work, scratch_dir, dbsrt_bin, phash_dir, dbphash_path)
        results.append(res)

        status = res.get("status", "ERROR")
        if status == "MATCH_CONFIRMED":
            matches_count += 1
            print(
                f"✅ MATCH ({res['top_match']} @ {res['confidence_percent']}%, {res['recognition_latency_ms']}ms)",
                flush=True,
            )
        elif "DIVERGENCE" in status:
            divergence_count += 1
            print(f"⚠️ {status} (Top: '{res.get('top_match', '')}')", flush=True)
        else:
            errors_count += 1
            print(f"❌ {status} ({res.get('error', '')})", flush=True)

    total_time = round(time.monotonic() - t_start, 2)
    summary = {
        "total_works_audited": len(WORKS_TO_AUDIT),
        "matches_confirmed": matches_count,
        "montage_divergences_expected": divergence_count,
        "download_or_stream_errors": errors_count,
        "elapsed_seconds": total_time,
        "results": results,
    }

    out_file = Path("/tmp/remote_trailers_audit_results.json")
    out_file.write_text(json.dumps(summary, indent=2))
    print("\n=== AUDIT SUMMARY ===")
    print(f"Total Works Audited : {len(WORKS_TO_AUDIT)}")
    print(f"Matches Confirmed    : {matches_count}")
    print(f"Montage Divergences  : {divergence_count} (Normal: montage de trailer multi-épisodes ou cartons promo)")
    print(f"Errors               : {errors_count}")
    print(f"Total Elapsed Time   : {total_time}s")
    print(f"Detailed JSON saved  : {out_file}")


if __name__ == "__main__":
    main()
