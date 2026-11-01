#!/usr/bin/env python3
"""Renames legacy <media file>.<fps>fps.phash to <moviehash>.<fps>fps.phash and writes its <moviehash>.json sidecar.

Runs on o2switch from the dbsrt directory, next to the batch worker whose FTP helpers it reuses.
Dry run unless --apply. Prints counts only: never a media name, a NAS host or a path.
"""

import argparse
import json
import os
import signal
import sys
import unicodedata
import urllib.parse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Callable, Dict, List, Tuple

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from scripts.o2switch_batch_worker import (  # noqa: E402
    DEFAULT_FTP_PORT,
    DEFAULT_NAS_HOST,
    DEFAULT_NAS_USER,
    DEFAULT_PHASH_DIR,
    MOVIEHASH_RE,
    PHASH_FPS_SUFFIX_RE,
    VideoTarget,
    list_nas_videos_ftp,
    resolve_nas_password,
    target_moviehash,
)

Move = Tuple[Path, Path, str]


def _classify(
    path: Path, by_name: Dict[str, List[VideoTarget]], moviehash_of: Callable[[VideoTarget], str | None]
) -> Tuple[str, Move | None]:
    """Why a .phash stays as it is, or ("to_migrate", its move); the target may still exist on disk."""
    base = path.name.removesuffix(".phash")
    fps = PHASH_FPS_SUFFIX_RE.search(base)
    if MOVIEHASH_RE.fullmatch(base.split(".", 1)[0]):
        return "already_moviehash", None
    if path.stat().st_size == 0:
        return "empty", None
    if fps is None:
        return "no_fps_suffix", None
    media_name = unicodedata.normalize("NFC", urllib.parse.unquote(base[: fps.start()]))
    videos = by_name.get(media_name, [])
    if len(videos) != 1:
        return ("ambiguous_on_nas" if videos else "not_on_nas"), None
    moviehash = moviehash_of(videos[0])
    if moviehash is None:
        return "no_moviehash", None
    return "to_migrate", (path, path.with_name(f"{moviehash}{fps.group(0)}.phash"), media_name)


def plan_migration(
    phash_dir: Path,
    nas_videos: List[VideoTarget],
    moviehash_of: Callable[[VideoTarget], str | None],
    jobs: int = 1,
) -> Tuple[List[Move], Counter]:
    """(legacy file, moviehash file, media name) for each legacy .phash found exactly once on the NAS.

    The moviehash reads wait on the network: `jobs` of them run at once, the result keeps the file order.
    """
    by_name: Dict[str, List[VideoTarget]] = {}
    for video in nas_videos:
        by_name.setdefault(video.filename, []).append(video)
    moves: List[Move] = []
    counts: Counter = Counter()
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        classified = list(pool.map(lambda p: _classify(p, by_name, moviehash_of), sorted(phash_dir.glob("*.phash"))))
    for reason, move in classified:
        if move is not None and (move[1].exists() or any(move[1] == planned[1] for planned in moves)):
            reason, move = "target_exists", None
        if move is not None:
            moves.append(move)
        counts[reason] += 1
    return moves, counts


def _write_sidecar(new: Path, media_name: str, frames: int, extracted_at: int) -> None:
    """Same fields as PhashSidecar (dbsrt-cli shared.rs); an existing sidecar of that moviehash is kept."""
    moviehash, rest = new.name.split(".", 1)
    sidecar = new.with_name(f"{moviehash}.json")
    if sidecar.exists():
        return
    fps = float(rest.removesuffix(".phash").removesuffix("fps"))
    tmp = sidecar.with_suffix(".json.tmp")
    payload = {
        "moviehash": moviehash,
        "media_name": media_name,
        "fps": fps,
        "frames": frames,
        "extracted_at": extracted_at,
    }
    tmp.write_text(json.dumps(payload, indent=2, ensure_ascii=False), encoding="utf-8")
    os.replace(tmp, sidecar)


def apply_moves(moves: List[Move]) -> int:
    """Writes each sidecar then renames the legacy file; a target that appeared since planning is left alone."""
    migrated = 0
    for old, new, media_name in moves:
        if new.exists() or not old.exists():
            continue
        stat = old.stat()
        _write_sidecar(new, media_name, stat.st_size // 8, int(stat.st_mtime))
        os.rename(old, new)
        migrated += 1
    return migrated


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true", help="Rename the files (default: dry run)")
    parser.add_argument("--phash-dir", default=DEFAULT_PHASH_DIR)
    parser.add_argument(
        "--nas-base", action="append", help="NAS base directory (repeatable, default /video and /tvshow)"
    )
    parser.add_argument("--jobs", type=int, default=16, help="Moviehash reads run at once (NAS took 64 sessions)")
    args = parser.parse_args()
    # The worker import installs handlers that log to the batch worker log and leave the pool running
    for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGQUIT):
        signal.signal(sig, signal.SIG_DFL)
    signal.signal(signal.SIGINT, signal.default_int_handler)

    nas_pass = resolve_nas_password()
    nas_videos: List[VideoTarget] = []
    for base in args.nas_base or ["/video", "/tvshow"]:
        nas_videos += list_nas_videos_ftp(DEFAULT_NAS_HOST, DEFAULT_NAS_USER, nas_pass, base_dir=base)
    ftp_env = {"DBSRT_FTP_USER": DEFAULT_NAS_USER, "DBSRT_FTP_PASSWORD": nas_pass}
    stream_base = f"ftp://{DEFAULT_NAS_HOST}:{DEFAULT_FTP_PORT}"

    moves, counts = plan_migration(
        Path(args.phash_dir), nas_videos, lambda video: target_moviehash(video, stream_base, ftp_env), args.jobs
    )
    if args.apply:
        counts["migrated"] = apply_moves(moves)
    # key=value lines, unquoted, so the output stays readable through quote-masking filters
    print(f"mode={'apply' if args.apply else 'dry-run'}")
    for key, value in sorted({"nas_videos": len(nas_videos), **counts}.items()):
        print(f"{key}={value}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
