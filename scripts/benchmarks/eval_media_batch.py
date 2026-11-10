#!/usr/bin/env python3
"""Generic batch media evaluation runner for pydbsrt.

Discovers media files dynamically, inspects container and sidecar subtitles,
tests server search and subtitle decoding integrity, and generates a structured
evaluation report without hardcoded paths or release lists.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any

DEFAULT_SERVER_URL: str = "http://127.0.0.1:8765"
DEFAULT_GLOB_PATTERN: str = "*.mkv"
DEFAULT_BATCH_LIMIT: int = 20
DEFAULT_TARGET_LANG: str = "fr"
DEFAULT_REPORT_FILENAME: str = "eval_batch_report.json"
HTTP_TIMEOUT_SECONDS: float = 10.0
FFPROBE_TIMEOUT_SECONDS: float = 15.0
UNICODE_REPLACEMENT_CHAR: str = "\ufffd"


@dataclass
class MediaEvalResult:
    """Individual media evaluation entry."""

    file_path: str
    stem: str
    duration_seconds: float = 0.0
    video_codec: str = "unknown"
    container_streams_count: int = 0
    embedded_subtitles: list[dict[str, Any]] = field(default_factory=list)
    sidecar_subtitles: list[str] = field(default_factory=list)
    search_status_code: int = 0
    candidates_count: int = 0
    decoding_clean: bool = True
    corrupted_char_count: int = 0
    error: str | None = None


def probe_media_file(media_path: Path) -> dict[str, Any]:
    """Inspects container metadata and streams via ffprobe."""
    cmd = [
        "ffprobe",
        "-v",
        "error",
        "-show_entries",
        "format=duration:stream=index,codec_name,codec_type:stream_tags=language,title",
        "-of",
        "json",
        str(media_path),
    ]
    try:
        proc = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=FFPROBE_TIMEOUT_SECONDS,
            check=False,
        )
        if proc.returncode == 0 and proc.stdout.strip():
            return json.loads(proc.stdout)
    except (subprocess.SubprocessError, json.JSONDecodeError, OSError):
        pass
    return {}


def discover_sidecar_subtitles(media_path: Path) -> list[Path]:
    """Finds adjacent or subdirectory .srt files matching the media stem."""
    stem = media_path.stem
    parent_dir = media_path.parent
    found: list[Path] = []

    # Search in immediate parent
    for candidate in parent_dir.glob("*.srt"):
        if candidate.is_file():
            found.append(candidate)

    # Search in Subs / subtitles subdirectories
    for subdir_name in ("Subs", "subs", "Subtitles", "subtitles"):
        sub_path = parent_dir / subdir_name
        if sub_path.is_dir():
            for candidate in sub_path.glob("*.srt"):
                if candidate.is_file():
                    found.append(candidate)

    # Prioritize files matching stem
    matching = [p for p in found if stem.lower() in p.stem.lower()]
    return matching if matching else found


def query_search_endpoint(
    server_url: str,
    media_stem: str,
    lang: str,
) -> tuple[int, list[dict[str, Any]]]:
    """Queries /v1/search for subtitle candidates."""
    url = f"{server_url.rstrip('/')}/v1/search?query={urllib.parse.quote(media_stem)}&lang={urllib.parse.quote(lang)}"
    req = urllib.request.Request(url, headers={"User-Agent": "pydbsrt-eval/1.0"})
    try:
        with urllib.request.urlopen(req, timeout=HTTP_TIMEOUT_SECONDS) as resp:
            data = json.loads(resp.read().decode("utf-8"))
            results = data.get("results", [])
            return resp.status, results
    except urllib.error.HTTPError as e:
        return e.code, []
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError):
        return 0, []


def verify_srt_decoding(srt_path: Path) -> tuple[bool, int]:
    """Verifies that an .srt file decodes cleanly without replacement characters."""
    try:
        raw_bytes = srt_path.read_bytes()
        # Test UTF-8 first
        try:
            content = raw_bytes.decode("utf-8")
        except UnicodeDecodeError:
            # Fallback to Latin-1
            content = raw_bytes.decode("latin-1")
        bad_chars = content.count(UNICODE_REPLACEMENT_CHAR)
        return bad_chars == 0, bad_chars
    except OSError:
        return False, -1


def evaluate_media_batch(
    input_dir: Path,
    pattern: str,
    limit: int,
    target_lang: str,
    server_url: str,
) -> list[MediaEvalResult]:
    """Runs batch evaluation across discovered media files."""
    results: list[MediaEvalResult] = []

    print(f"Scanning '{input_dir}' for pattern '{pattern}' (limit: {limit})...")
    discovered_files: list[Path] = []
    for ext in pattern.split(","):
        clean_ext = ext.strip()
        discovered_files.extend(input_dir.rglob(clean_ext))

    # Sort deterministically
    discovered_files.sort()
    selected_files = discovered_files[:limit]
    print(f"Discovered {len(discovered_files)} files; evaluating first {len(selected_files)}.")

    for idx, media_path in enumerate(selected_files, start=1):
        print(f"[{idx}/{len(selected_files)}] Inspecting: {media_path.name}")
        entry = MediaEvalResult(
            file_path=str(media_path),
            stem=media_path.stem,
        )

        probe_data = probe_media_file(media_path)
        fmt = probe_data.get("format", {})
        try:
            entry.duration_seconds = float(fmt.get("duration", 0.0))
        except (ValueError, TypeError):
            entry.duration_seconds = 0.0

        streams = probe_data.get("streams", [])
        entry.container_streams_count = len(streams)
        for s in streams:
            codec_type = s.get("codec_type")
            if codec_type == "video" and entry.video_codec == "unknown":
                entry.video_codec = s.get("codec_name", "unknown")
            elif codec_type == "subtitle":
                entry.embedded_subtitles.append(
                    {
                        "index": s.get("index"),
                        "codec": s.get("codec_name"),
                        "lang": s.get("tags", {}).get("language", "und"),
                        "title": s.get("tags", {}).get("title"),
                    }
                )

        sidecars = discover_sidecar_subtitles(media_path)
        entry.sidecar_subtitles = [str(s) for s in sidecars]

        for srt_path in sidecars:
            clean, bad_count = verify_srt_decoding(srt_path)
            if not clean:
                entry.decoding_clean = False
                entry.corrupted_char_count += bad_count

        status_code, candidates = query_search_endpoint(server_url, media_path.stem, target_lang)
        entry.search_status_code = status_code
        entry.candidates_count = len(candidates)

        results.append(entry)

    return results


def main() -> int:
    """CLI entrypoint."""
    parser = argparse.ArgumentParser(
        description="Dynamic batch media evaluation tool for pydbsrt",
    )
    parser.add_argument(
        "--input-dir",
        type=Path,
        default=Path(os.environ.get("MEDIA_DIR", ".")),
        help="Root directory to search for media files (default: $MEDIA_DIR or current directory)",
    )
    parser.add_argument(
        "--pattern",
        type=str,
        default=DEFAULT_GLOB_PATTERN,
        help=f"Glob pattern or comma-separated patterns (default: {DEFAULT_GLOB_PATTERN})",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=DEFAULT_BATCH_LIMIT,
        help=f"Maximum number of files to process (default: {DEFAULT_BATCH_LIMIT})",
    )
    parser.add_argument(
        "--lang",
        type=str,
        default=DEFAULT_TARGET_LANG,
        help=f"Target subtitle language code (default: {DEFAULT_TARGET_LANG})",
    )
    parser.add_argument(
        "--server-url",
        type=str,
        default=DEFAULT_SERVER_URL,
        help=f"Base URL of dbsrt-server (default: {DEFAULT_SERVER_URL})",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=Path(DEFAULT_REPORT_FILENAME),
        help=f"Path to write JSON evaluation report (default: {DEFAULT_REPORT_FILENAME})",
    )

    args = parser.parse_args()

    if not args.input_dir.exists():
        print(f"Error: Input directory '{args.input_dir}' does not exist.", file=sys.stderr)
        return 1

    results = evaluate_media_batch(
        input_dir=args.input_dir,
        pattern=args.pattern,
        limit=args.limit,
        target_lang=args.lang,
        server_url=args.server_url,
    )

    summary = {
        "input_dir": str(args.input_dir),
        "total_evaluated": len(results),
        "total_with_sidecars": sum(1 for r in results if r.sidecar_subtitles),
        "total_with_embedded": sum(1 for r in results if r.embedded_subtitles),
        "decoding_failures": sum(1 for r in results if not r.decoding_clean),
        "entries": [asdict(r) for r in results],
    }

    args.output.write_text(json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8")
    print(f"\n✓ Evaluation complete. Summary written to {args.output}")
    print(
        f"  Total evaluated: {summary['total_evaluated']}, "
        f"Sidecars: {summary['total_with_sidecars']}, "
        f"Embedded: {summary['total_with_embedded']}, "
        f"Corruptions: {summary['decoding_failures']}"
    )

    return 0


if __name__ == "__main__":
    sys.exit(main())
