#!/usr/bin/env python3
"""Generic batch media evaluation runner for pydbsrt.

Discovers media files dynamically, inspects container and sidecar subtitles,
tests server search and subtitle decoding integrity, and generates a structured
evaluation report without hardcoded paths or release lists.
"""

from __future__ import annotations

import argparse
import json
import shutil
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


def find_repo_root() -> Path:
    """Dynamically locates repository root with fallback to ~/dbsrt."""
    current = Path(__file__).resolve().parent
    for p in [current, *current.parents]:
        if (p / ".git").is_dir() or (p / "Taskfile.yml").is_file():
            return p
    appliance = Path.home() / "dbsrt"
    if appliance.is_dir():
        return appliance
    return Path.cwd()


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
    ffprobe_bin = shutil.which("ffprobe") or "ffprobe"
    cmd = [
        ffprobe_bin,
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
    found: list[Path] = [p for p in parent_dir.glob("*.srt") if p.is_file()]

    for subdir_name in ("Subs", "subs", "Subtitles", "subtitles"):
        sub_path = parent_dir / subdir_name
        if sub_path.is_dir():
            found.extend(p for p in sub_path.glob("*.srt") if p.is_file())

    matching = [p for p in found if stem.lower() in p.stem.lower()]
    return matching if matching else found


def verify_srt_decoding(srt_path: Path) -> tuple[bool, int]:
    """Verifies that an SRT file can be decoded cleanly as UTF-8 or CP1252 without corrupted characters."""
    try:
        raw_bytes = srt_path.read_bytes()
    except OSError:
        return False, 0

    try:
        text = raw_bytes.decode("utf-8")
        corrupted_count = text.count(UNICODE_REPLACEMENT_CHAR)
        return (corrupted_count == 0), corrupted_count
    except UnicodeDecodeError:
        pass

    try:
        raw_bytes.decode("cp1252")
        return True, 0
    except UnicodeDecodeError:
        return False, 1


def query_search_endpoint(server_url: str, stem: str, target_lang: str) -> tuple[int, list[dict[str, Any]]]:
    """Queries dbsrt-server search endpoint for candidates."""
    url = f"{server_url.rstrip('/')}/v1/search"
    payload = json.dumps({"media_path": stem, "language": target_lang}).encode("utf-8")
    req = urllib.request.Request(
        url,
        data=payload,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=HTTP_TIMEOUT_SECONDS) as resp:
            data = json.loads(resp.read().decode("utf-8"))
            return resp.status, data.get("candidates", [])
    except urllib.error.HTTPError as e:
        return e.code, []
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError, OSError):
        return 0, []


def _evaluate_single_media(
    media_path: Path,
    server_url: str,
    target_lang: str,
) -> MediaEvalResult:
    """Inspects and evaluates a single media file."""
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
    return entry


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

    discovered_files.sort()
    selected_files = discovered_files[:limit]
    print(f"Discovered {len(discovered_files)} files; evaluating first {len(selected_files)}.")

    for idx, media_path in enumerate(selected_files, start=1):
        print(f"[{idx}/{len(selected_files)}] Inspecting: {media_path.name}")
        entry = _evaluate_single_media(media_path, server_url, target_lang)
        results.append(entry)

    return results


def _run_inspection_mode(
    repo_root: Path,
    input_dir: Path,
    pattern: str,
    server_url: str,
    output_path: Path,
) -> int:
    """Print environment and input summary for check mode."""
    ffprobe_bin = shutil.which("ffprobe")
    print("=== DBSrt Batch Media Evaluation Inspection ===")
    print(f"✓ REPO_ROOT     : {repo_root}")
    print(f"✓ Input dir     : {input_dir} (exists: {input_dir.exists()})")
    matching: list[Path] = []
    if input_dir.is_dir():
        for ext in pattern.split(","):
            matching.extend(input_dir.rglob(ext.strip()))
    print(f"✓ Matching files: {len(matching)} file(s) found for pattern '{pattern}'")
    print(f"✓ ffprobe binary: {ffprobe_bin or 'not found'}")
    print(f"✓ Server URL    : {server_url}")
    print(f"✓ Output report : {output_path}")
    return 0


def main() -> int:
    """CLI entrypoint."""
    repo_root = find_repo_root()
    default_input_dir = repo_root / "data/snippets" if (repo_root / "data/snippets").is_dir() else repo_root

    parser = argparse.ArgumentParser(
        description="Dynamic batch media evaluation tool for pydbsrt",
    )
    parser.add_argument(
        "--input-dir",
        type=Path,
        default=default_input_dir,
        help=f"Root directory to search for media files (default: {default_input_dir})",
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
    parser.add_argument(
        "--check",
        "--dry-run",
        action="store_true",
        help="Inspect input directory, tools, and pattern match count without executing batch",
    )
    args = parser.parse_args()

    input_dir = args.input_dir
    if not input_dir.is_absolute() and not input_dir.exists():
        input_dir = repo_root / input_dir

    output_path = args.output
    if not output_path.is_absolute():
        output_path = repo_root / output_path

    if args.check:
        return _run_inspection_mode(repo_root, input_dir, args.pattern, args.server_url, output_path)

    if not input_dir.exists():
        print(f"Error: Input directory '{input_dir}' does not exist.", file=sys.stderr)
        return 1

    results = evaluate_media_batch(
        input_dir=input_dir,
        pattern=args.pattern,
        limit=args.limit,
        target_lang=args.lang,
        server_url=args.server_url,
    )

    summary = {
        "input_dir": str(input_dir),
        "total_evaluated": len(results),
        "total_with_sidecars": sum(1 for r in results if r.sidecar_subtitles),
        "total_with_embedded": sum(1 for r in results if r.embedded_subtitles),
        "decoding_failures": sum(1 for r in results if not r.decoding_clean),
        "entries": [asdict(r) for r in results],
    }

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(summary, indent=2, ensure_ascii=False), encoding="utf-8")
    print(f"\n✓ Evaluation complete. Summary written to {output_path}")
    print(
        f"  Total evaluated: {summary['total_evaluated']}, "
        f"Sidecars: {summary['total_with_sidecars']}, "
        f"Embedded: {summary['total_with_embedded']}, "
        f"Corruptions: {summary['decoding_failures']}"
    )

    return 0


if __name__ == "__main__":
    sys.exit(main())
