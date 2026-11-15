"""CLI argument parsing, target selection pipeline, and main entry point for the batch worker."""

from __future__ import annotations

import argparse
import os
import sys
import urllib.parse
from pathlib import Path
from typing import List, Set

from ._facade import facade_attr
from .discovery import (
    is_excluded,
    list_nas_videos_ftp,
    load_blacklisted_stems,
    load_excluded_prefixes,
    load_targets_file,
    match_nas_paths,
    resolve_retry_targets,
)
from .models import (
    CPU_BUDGET_VCPUS,
    DEFAULT_AHASH_DIR,
    DEFAULT_COUPLINGS_PATH,
    DEFAULT_EVENTS_LOG_PATH,
    DEFAULT_EXCLUDE_PATH,
    DEFAULT_FTP_PORT,
    DEFAULT_LOG_PATH,
    DEFAULT_MAX_EXTRACTION_SECS,
    DEFAULT_NAS_BASE,
    DEFAULT_NAS_HOST,
    DEFAULT_NAS_USER,
    DEFAULT_PHASH_DIR,
    DEFAULT_TARGETS_PATH,
    DEFAULT_WORKERS,
    MAX_FAILED_ATTEMPTS,
    REPO_ROOT,
    VideoTarget,
    resolve_nas_password,
)
from .runner import RELOAD_REQUESTED, build_child_env, install_process_handlers
from .scheduler import execute_dynamic_batch_ingestion
from .stems import (
    load_certified_stems,
    load_existing_ahash_stems,
    load_existing_phash_stems,
    resolve_done_stems,
)
from .telemetry import redact_nas, write_batch_queue_state


def build_arg_parser() -> argparse.ArgumentParser:
    """Constructs the CLI parser for `o2switch_batch_worker.py`."""
    parser = argparse.ArgumentParser(description="o2switch Remote Batch Processing Worker")
    parser.add_argument("--workers", type=int, default=DEFAULT_WORKERS, help="Parallel workers")
    parser.add_argument("--ftp-port", type=int, default=DEFAULT_FTP_PORT, help="NAS FTP port")
    parser.add_argument(
        "--fresh-only",
        dest="fresh_only",
        action="store_true",
        default=True,
        help="Process only unindexed videos missing required fingerprints (default: enabled)",
    )
    parser.add_argument(
        "--force-all",
        "--no-fresh-only",
        dest="fresh_only",
        action="store_false",
        help="Reprocess all discovered videos even when .phash/.ahash already exist",
    )
    parser.add_argument("--retry-failed", action="store_true", help="Retry only videos whose last run failed")
    parser.add_argument(
        "--max-extraction-secs",
        type=int,
        default=DEFAULT_MAX_EXTRACTION_SECS,
        help=f"Maximum extraction wall duration before watchdog kill (default: {DEFAULT_MAX_EXTRACTION_SECS}s)",
    )
    parser.add_argument("--limit", type=int, default=None, help="Maximum number of videos to process")
    parser.add_argument("--nas-host", default=DEFAULT_NAS_HOST, help="Synology NAS DDNS or IP (env NAS_HOST)")
    parser.add_argument("--nas-user", default=DEFAULT_NAS_USER, help="Synology NAS FTP username (env NAS_USER)")
    parser.add_argument("--nas-base", default=DEFAULT_NAS_BASE, help="NAS base directory")
    parser.add_argument("--stream-base", default=None, help="Explicit stream base URL (HTTP or FTP)")
    parser.add_argument("--targets-file", default=DEFAULT_TARGETS_PATH, help="JSON file listing target video paths")
    parser.add_argument("--mode", choices=["phash", "ahash", "both"], default="both", help="Batch processing mode")
    parser.add_argument("--ahash-dir", default=DEFAULT_AHASH_DIR, help="Directory storing .ahash fingerprints")
    parser.add_argument(
        "--check", "--dry-run", dest="check", action="store_true", help="Inspect environment without executing batch"
    )
    return parser


def _handle_check_mode(args: argparse.Namespace, dbsrt_bin: str) -> int:
    """Prints environment inspection summary and exits with code 0."""
    pw_fn = facade_attr("resolve_nas_password", resolve_nas_password)
    nas_pass = pw_fn() if not args.stream_base else ""
    has_nas_creds = bool(args.nas_host and args.nas_user and nas_pass)
    repo_root = facade_attr("REPO_ROOT", REPO_ROOT)
    couplings_file = repo_root / facade_attr("DEFAULT_COUPLINGS_PATH", DEFAULT_COUPLINGS_PATH)
    phash_dir = repo_root / facade_attr("DEFAULT_PHASH_DIR", DEFAULT_PHASH_DIR)
    ahash_dir = repo_root / args.ahash_dir
    budget = facade_attr("CPU_BUDGET_VCPUS", CPU_BUDGET_VCPUS)
    print("=== DBSrt o2switch Batch Worker Inspection ===")
    print(f"✓ REPO_ROOT     : {repo_root}")
    print(f"✓ Binary dbsrt  : {dbsrt_bin} ({'found' if Path(dbsrt_bin).exists() else 'missing'})")
    print(f"✓ Mode          : {args.mode}")
    print(f"✓ Workers budget: {args.workers} workers (budget: {budget} vCPUs)")
    print(f"✓ Couplings path: {couplings_file} ({'found' if couplings_file.is_file() else 'missing'})")
    print(f"✓ pHash dir     : {phash_dir} ({'found' if phash_dir.is_dir() else 'missing'})")
    print(f"✓ aHash dir     : {ahash_dir} ({'found' if ahash_dir.is_dir() else 'missing'})")
    print(f"✓ NAS config    : {'configured' if has_nas_creds else 'unconfigured (stream mode required)'}")
    print(f"✓ Log path      : {repo_root / DEFAULT_LOG_PATH}")
    return 0


def _narrow_to_targets_file(
    args: argparse.Namespace,
    targets: List[VideoTarget],
    all_videos: List[VideoTarget],
) -> List[VideoTarget]:
    """Narrows retry candidates to an explicit non-default `--targets-file` when supplied."""
    default_targets = facade_attr("DEFAULT_TARGETS_PATH", DEFAULT_TARGETS_PATH)
    if args.targets_file == default_targets or not Path(args.targets_file).is_file():
        return targets
    listed = match_nas_paths(load_targets_file(Path(args.targets_file), "", set()), all_videos)
    wanted = {video.filename for video in listed}
    return [video for video in targets if video.filename in wanted]


def _initial_targets(
    args: argparse.Namespace,
    all_videos: List[VideoTarget],
    events_log: Path,
    done_stems: Set[str],
    skip_stems: Set[str],
) -> List[VideoTarget]:
    """Resolves initial target list from stream targets, failed retries, targets file, or NAS scan."""
    if args.stream_base:
        targets_path = Path(args.targets_file)
        if targets_path.is_file():
            targets = load_targets_file(targets_path, "", skip_stems)
            print(f"✓ Loaded {len(targets)} targets from {targets_path} (stream mode).")
            return targets
        print(f"❌ Error: --stream-base requires a valid targets file ({args.targets_file}).", file=sys.stderr)
        return []
    if args.retry_failed:
        targets = _narrow_to_targets_file(
            args,
            match_nas_paths(resolve_retry_targets(events_log, done_stems), all_videos),
            all_videos,
        )
        print(f"✓ Found {len(targets)} failed videos to retry under {args.nas_base}.")
        return targets
    targets_path = Path(args.targets_file)
    if targets_path.is_file():
        try:
            targets = match_nas_paths(load_targets_file(targets_path, args.nas_base, skip_stems), all_videos)
            print(f"✓ Loaded {len(targets)} targets from {targets_path} found on the NAS.")
            return targets
        except Exception as err:
            print(f"⚠️ Failed reading {targets_path}: {err}. Falling back to remote scan.", file=sys.stderr)
    return [video for video in all_videos if Path(video.filename).stem not in skip_stems]


def select_targets(args: argparse.Namespace, nas_pass: str, events_log: Path) -> List[VideoTarget]:
    """Scans local artifacts, skips done/blacklisted stems, and selects pending remote targets."""
    print("=== [1/4] Scanning local catalogs and couplings ===")
    couplings_path = Path(facade_attr("DEFAULT_COUPLINGS_PATH", DEFAULT_COUPLINGS_PATH))
    phash_dir = Path(facade_attr("DEFAULT_PHASH_DIR", DEFAULT_PHASH_DIR))
    done_stems = resolve_done_stems(
        args.mode,
        load_certified_stems(couplings_path),
        load_existing_phash_stems(phash_dir),
        load_existing_ahash_stems(Path(args.ahash_dir)),
    )

    blacklisted = load_blacklisted_stems(events_log)
    print(f"✓ Skipping {len(blacklisted)} videos that failed {MAX_FAILED_ATTEMPTS} times in a row.")
    skip_stems = (done_stems if args.fresh_only else set()) | blacklisted

    if args.stream_base:
        all_videos: List[VideoTarget] = []
    else:
        print(f"=== [2/4] Discovering remote NAS video library (<nas>:{args.nas_base}) ===")
        ftp_fn = facade_attr("list_nas_videos_ftp", list_nas_videos_ftp)
        all_videos = ftp_fn(
            nas_host=args.nas_host,
            nas_user=args.nas_user,
            nas_pass=nas_pass,
            base_dir=args.nas_base,
            ftp_port=args.ftp_port,
            bypass_index_cache=bool(args.fresh_only),
        )
        print(f"✓ Discovered {len(all_videos)} video files on NAS.")

    targets = _initial_targets(args, all_videos, events_log, done_stems, skip_stems)
    exclude_path = Path(facade_attr("DEFAULT_EXCLUDE_PATH", DEFAULT_EXCLUDE_PATH))
    excluded = load_excluded_prefixes(exclude_path)
    if excluded:
        kept = [video for video in targets if not is_excluded(video.rel_path, excluded)]
        print(f"✓ Excluded {len(targets) - len(kept)} videos listed in {exclude_path}.")
        targets = kept
    return targets[: args.limit] if args.limit else targets


def _resolve_dbsrt_bin() -> str:
    """Resolves `dbsrt` executable under `bin/` or `target/`."""
    repo_root = facade_attr("REPO_ROOT", REPO_ROOT)
    for cand in (repo_root / "bin/dbsrt", repo_root / "target/release/dbsrt", repo_root / "target/debug/dbsrt"):
        if cand.exists():
            return str(cand)
    return "dbsrt"


def _validate_nas_credentials(args: argparse.Namespace, nas_pass: str) -> bool:
    """Verifies NAS credentials when running in direct FTP discovery mode."""
    if args.stream_base:
        return True
    missing = [
        k for k, v in (("NAS_HOST", args.nas_host), ("NAS_USER", args.nas_user), ("NAS_PASS", nas_pass)) if not v
    ]
    if missing:
        print(
            f"❌ Error: missing NAS config {', '.join(missing)} (env, etc/.nas_config or etc/.nas_pass).",
            file=sys.stderr,
        )
        return False
    return True


def _run_ingestion_dispatch(
    targets: List[VideoTarget],
    stream_base: str,
    dbsrt_bin: str,
    child_env: dict[str, str],
    events_log: Path,
    args: argparse.Namespace,
) -> tuple[int, int, float]:
    ingest_fn = facade_attr("execute_dynamic_batch_ingestion", execute_dynamic_batch_ingestion)
    kwargs = {
        "targets": targets,
        "stream_base": stream_base,
        "dbsrt_bin": dbsrt_bin,
        "child_env": child_env,
        "events_log": events_log,
        "max_extraction_secs": args.max_extraction_secs,
        "initial_workers": args.workers,
        "ahash_dir": args.ahash_dir,
    }
    try:
        return ingest_fn(**kwargs, mode=args.mode)
    except TypeError:
        return ingest_fn(**kwargs)  # type: ignore[call-arg]


def main() -> int:
    """CLI entry point for `o2switch` remote batch processing."""
    install_process_handlers()
    sys_mod = facade_attr("sys", sys)
    args = build_arg_parser().parse_args(sys_mod.argv[1:])
    dbsrt_bin = _resolve_dbsrt_bin()
    if args.check:
        return _handle_check_mode(args, dbsrt_bin)

    pw_fn = facade_attr("resolve_nas_password", resolve_nas_password)
    nas_pass = pw_fn() if not args.stream_base else ""
    if not _validate_nas_credentials(args, nas_pass):
        return 1

    events_log = Path(DEFAULT_EVENTS_LOG_PATH)
    targets = select_targets(args, nas_pass, events_log)
    print(f"✓ Selected {len(targets)} target videos to process (fresh_only={args.fresh_only}, limit={args.limit}).")
    if not targets:
        print("🎉 No target videos to process. Library is up-to-date!")
        facade_attr("write_batch_queue_state", write_batch_queue_state)(0, 0, 0, 0)
        return 0

    if args.stream_base:
        stream_base = args.stream_base.rstrip("/")
        nas_host = urllib.parse.urlsplit(stream_base).hostname or ""
        print(f"=== [3/4] Direct HTTP Native Streaming Mode ({redact_nas(stream_base, nas_host, '')}) ===")
    else:
        print(f"=== [3/4] Direct FTP Native Streaming Mode (<nas>:{args.ftp_port}) ===")
        stream_base = f"ftp://{args.nas_host}:{args.ftp_port}"
    child_env = build_child_env(args.nas_user, nas_pass, Path("bin").resolve())

    print(f"=== [4/4] Executing Batch Ingestion ({args.workers} initial parallel workers, dynamic scaling active) ===")
    success_count, fail_count, total_duration = _run_ingestion_dispatch(
        targets, stream_base, dbsrt_bin, child_env, events_log, args
    )

    if RELOAD_REQUESTED.is_set():
        print(f"🔄 [RELOAD] Re-exec after {success_count} ok / {fail_count} failed", flush=True)
        facade_attr("os", os).execv(sys_mod.executable, sys_mod.orig_argv)
        return 0

    print("=" * 60)
    print(f"🏁 Batch completed in {total_duration:.1f}s: {success_count} succeeded, {fail_count} failed.")
    return 0 if fail_count == 0 else 1
