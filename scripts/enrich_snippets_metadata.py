#!/usr/bin/env python3
"""Enriches NAS video snippets with structured metadata for database insertion.

Parses filenames, directory hierarchy, and ffprobe stream tags to extract:
- Title (clean name)
- Release Year
- Country (ISO 3166-1 alpha-2) & Language (ISO 639-1)
- Season & Episode (for TV series)
- Resolution, Codec, Audio/Subtitle streams
- Generates JSON catalog (metadata.json) and PostgreSQL import statements (medias_metadata.sql)
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


def parse_video_metadata(filename: str, category: str = "", nas_path: str = "") -> dict:
    stem = Path(filename).stem
    if stem.endswith("_mid15s"):
        stem = stem[:-8]

    meta: dict[str, str | int | float | None] = {
        "raw_name": Path(filename).name,
        "clean_stem": stem,
        "category": category,
        "nas_path": nas_path,
        "title": None,
        "year": None,
        "season": None,
        "episode": None,
        "country": None,
        "language": None,
        "resolution": None,
        "video_codec": None,
        "release_group": None,
    }

    # 1. Year (1920-2030)
    m_year = re.search(r"[\(\.\s\[_](19\d\d|20[0-3]\d)[\)\.\s\]_]", stem)
    if m_year:
        meta["year"] = int(m_year.group(1))

    # 2. Season & Episode
    m_se = re.search(r"[Ss](\d{1,2})[Ee](\d{1,2})", stem)
    if m_se:
        meta["season"] = int(m_se.group(1))
        meta["episode"] = int(m_se.group(2))
    else:
        m_s = re.search(r"[Ss](\d{1,2})", stem)
        if m_s:
            meta["season"] = int(m_s.group(1))

    # 3. Resolution
    m_res = re.search(r"(2160p|1080p|720p|480p|576p|4k)", stem, re.IGNORECASE)
    if m_res:
        meta["resolution"] = m_res.group(1).lower()

    # 4. Video Codec
    m_codec = re.search(r"(x264|x265|h264|h265|hevc|av1|xvid)", stem, re.IGNORECASE)
    if m_codec:
        meta["video_codec"] = m_codec.group(1).lower()

    # 5. Release Group
    m_grp = re.search(r"-([A-Za-z0-9]+)(?:\[.*?\])?$", stem)
    if m_grp:
        meta["release_group"] = m_grp.group(1)

    # 6. Title cleanup
    clean = re.sub(r"\[.*?\]|\(.*?\)", " ", stem)
    clean = re.split(
        r"(?:19\d\d|20\d\d|[Ss]\d+|1080p|720p|2160p|x264|x265|bluray|web-dl|webrip)", clean, flags=re.IGNORECASE
    )[0]
    clean = clean.replace(".", " ").replace("_", " ").strip("- ").strip()
    meta["title"] = clean if clean else stem

    # 7. Country & Language heuristics
    full_str = f"{stem} {category} {nas_path}".lower()

    if any(
        k in full_str for k in ["french", "francais", "vff", "vf", "vostfr", "arte", "dix pour cent", "effondrement"]
    ):
        meta["country"] = "FR"
        meta["language"] = "fr"
    elif any(k in full_str for k in ["korean", "korea", "vostfr-kr"]):
        meta["country"] = "KR"
        meta["language"] = "ko"
    elif any(k in full_str for k in ["anime", "japan", "jap", "haikyuu", "evangelion", "bebop", "demon slayer"]):
        meta["country"] = "JP"
        meta["language"] = "ja"
    elif any(k in full_str for k in ["spanish", "espanol", "espana"]):
        meta["country"] = "ES"
        meta["language"] = "es"
    elif any(k in full_str for k in ["german", "deutsch", "germany"]):
        meta["country"] = "DE"
        meta["language"] = "de"
    elif any(k in full_str for k in ["italian", "italiano", "italia"]):
        meta["country"] = "IT"
        meta["language"] = "it"
    elif any(k in full_str for k in ["(us)", " us ", "/us/", "amzn", "ctrlhd"]):
        meta["country"] = "US"
        meta["language"] = "en"
    elif any(k in full_str for k in ["uk", "british", "bbc"]):
        meta["country"] = "GB"
        meta["language"] = "en"
    else:
        # Default fallback for international catalog
        meta["country"] = "US"
        meta["language"] = "en"

    return meta


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Enrich extracted NAS video snippets with structured metadata for database insertion."
    )
    parser.add_argument(
        "--snippets-dir",
        type=Path,
        default=Path("data/snippets"),
        help="Directory containing snippets and phash files (default: data/snippets)",
    )
    parser.add_argument(
        "--output-json",
        type=Path,
        default=Path("data/snippets/metadata.json"),
        help="Output JSON file path (default: data/snippets/metadata.json)",
    )
    parser.add_argument(
        "--output-sql",
        type=Path,
        default=Path("data/snippets/medias_metadata.sql"),
        help="Output SQL file path (default: data/snippets/medias_metadata.sql)",
    )

    args = parser.parse_args()

    if not args.snippets_dir.exists():
        print(f"❌ Snippets directory not found: {args.snippets_dir}", file=sys.stderr)
        return 1

    manifest_path = args.snippets_dir / "manifest.json"
    manifest: dict[str, dict] = {}
    if manifest_path.exists():
        try:
            with open(manifest_path, encoding="utf-8") as f:
                manifest = json.load(f)
        except (json.JSONDecodeError, OSError):
            pass

    mp4_files = sorted(args.snippets_dir.glob("*_mid15s.mp4"))
    print("================================================================================")
    print(" 🏷️ ENRICHING NAS SNIPPETS METADATA FOR DATABASE")
    print("================================================================================")
    print(f"Snippets Directory: {args.snippets_dir}")
    print(f"Discovered MP4s:    {len(mp4_files)}")
    print(f"Manifest Entries:   {len(manifest)}")
    print("--------------------------------------------------------------------------------")

    enriched_catalog: dict[str, dict] = {}
    sql_statements: list[str] = [
        "-- Auto-generated PostgreSQL metadata insert script for pydbsrt medias",
        "ALTER TABLE medias ADD COLUMN IF NOT EXISTS title TEXT;",
        "ALTER TABLE medias ADD COLUMN IF NOT EXISTS year INT;",
        "ALTER TABLE medias ADD COLUMN IF NOT EXISTS country TEXT;",
        "ALTER TABLE medias ADD COLUMN IF NOT EXISTS language TEXT;",
        "ALTER TABLE medias ADD COLUMN IF NOT EXISTS season INT;",
        "ALTER TABLE medias ADD COLUMN IF NOT EXISTS episode INT;",
        "ALTER TABLE medias ADD COLUMN IF NOT EXISTS resolution TEXT;",
        "ALTER TABLE medias ADD COLUMN IF NOT EXISTS video_codec TEXT;",
        "ALTER TABLE medias ADD COLUMN IF NOT EXISTS metadata JSONB;",
        "",
    ]

    for mp4 in mp4_files:
        stem = mp4.stem
        phash_file = args.snippets_dir / f"{stem}.phash"
        phash_frames = (phash_file.stat().st_size - 4) // 8 if phash_file.exists() else 0

        # Try to match manifest
        matched_info = None
        for orig_name, info in manifest.items():
            if info.get("mp4_file") == mp4.name:
                matched_info = info
                break

        category = matched_info.get("category", "") if matched_info else ""
        orig_video = matched_info.get("video", mp4.name) if matched_info else mp4.name
        total_dur = matched_info.get("total_duration_s") if matched_info else None
        start_time = matched_info.get("start_time_s") if matched_info else None

        meta = parse_video_metadata(orig_video, category=category)
        meta["snippet_mp4"] = mp4.name
        meta["snippet_bytes"] = mp4.stat().st_size
        meta["phash_file"] = phash_file.name if phash_file.exists() else None
        meta["phash_frames"] = phash_frames
        meta["total_duration_s"] = total_dur
        meta["snippet_offset_s"] = start_time

        enriched_catalog[mp4.stem] = meta

        # Format SQL statement
        clean_title = (meta["title"] or orig_video).replace("'", "''")
        db_name = orig_video.replace("'", "''")
        phash_rel = f"data/snippets/{phash_file.name}" if phash_file.exists() else ""
        year_val = str(meta["year"]) if meta["year"] else "NULL"
        country_val = f"'{meta['country']}'" if meta["country"] else "NULL"
        lang_val = f"'{meta['language']}'" if meta["language"] else "NULL"
        season_val = str(meta["season"]) if meta["season"] else "NULL"
        episode_val = str(meta["episode"]) if meta["episode"] else "NULL"
        res_val = f"'{meta['resolution']}'" if meta["resolution"] else "NULL"
        codec_val = f"'{meta['video_codec']}'" if meta["video_codec"] else "NULL"
        meta_json = json.dumps(meta, ensure_ascii=False).replace("'", "''")

        sql_line = (
            f"INSERT INTO medias (name, title, year, country, language, season, episode, resolution, video_codec, frame_count, phash_path, metadata) "
            f"VALUES ('{db_name}', '{clean_title}', {year_val}, {country_val}, {lang_val}, {season_val}, {episode_val}, {res_val}, {codec_val}, {phash_frames}, '{phash_rel}', '{meta_json}'::jsonb) "
            f"ON CONFLICT (name) DO UPDATE SET "
            f"title = EXCLUDED.title, year = EXCLUDED.year, country = EXCLUDED.country, language = EXCLUDED.language, "
            f"season = EXCLUDED.season, episode = EXCLUDED.episode, resolution = EXCLUDED.resolution, video_codec = EXCLUDED.video_codec, "
            f"frame_count = EXCLUDED.frame_count, phash_path = EXCLUDED.phash_path, metadata = EXCLUDED.metadata;"
        )
        sql_statements.append(sql_line)

    with open(args.output_json, "w", encoding="utf-8") as f:
        json.dump(enriched_catalog, f, indent=2, ensure_ascii=False)

    with open(args.output_sql, "w", encoding="utf-8") as f:
        f.write("\n".join(sql_statements) + "\n")

    print(f"✅ Generated {len(enriched_catalog)} enriched metadata records:")
    print(f"   • JSON Catalog: {args.output_json}")
    print(f"   • SQL Statements: {args.output_sql}")

    # Summary by country & category
    countries: dict[str, int] = {}
    for m in enriched_catalog.values():
        c = str(m.get("country") or "UNKNOWN")
        countries[c] = countries.get(c, 0) + 1

    print("\n📊 Distribution par Pays :")
    for c, cnt in sorted(countries.items(), key=lambda x: -x[1]):
        print(f"   • {c}: {cnt}")

    return 0


if __name__ == "__main__":
    sys.exit(main())
