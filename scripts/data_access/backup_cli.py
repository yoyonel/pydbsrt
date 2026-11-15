#!/usr/bin/env python3
"""CLI & orchestration engine for versioned SHA-256 dataset backups, eviction, and restore."""

from __future__ import annotations

import argparse
import json
import os
import sys
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

_REPO_ROOT = Path(__file__).resolve().parents[2]
if str(_REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(_REPO_ROOT))

from scripts.data_access.backup_manifest import (  # noqa: E402
    DATASET_SPEC_BY_ID,
    build_backup_manifest,
    compute_file_sha256,
    extract_manifest_summary,
    load_evicted_manifest,
    normalize_dataset_ids,
    save_evicted_manifest,
    save_latest_backup_summary_cache,
)
from scripts.data_access.backup_targets import (  # noqa: E402
    BackupTarget,
    build_configured_targets,
)
from scripts.data_access.constants import (  # noqa: E402
    DEFAULT_MIN_EVICT_REPLICAS,
    EXIT_OK,
    EXIT_STALE_OR_DRIFT,
    EXIT_UNAVAILABLE,
    EXIT_USAGE_ERROR,
)
from scripts.data_access.topology import resolve_workspace_context  # noqa: E402


def _log(msg: str) -> None:
    """Writes operational progress strictly to stderr, keeping stdout pure JSON."""
    print(msg, file=sys.stderr, flush=True)


def _reachable_targets(targets: list[BackupTarget]) -> tuple[list[BackupTarget], list[str]]:
    """Splits configured targets into reachable adapters and unreachable names."""
    reachable: list[BackupTarget] = []
    skipped: list[str] = []
    for tgt in targets:
        if tgt.is_reachable():
            reachable.append(tgt)
        else:
            skipped.append(tgt.name)
    return reachable, skipped


def run_manifest_cmd(
    data_dir: Path,
    datasets: list[str] | None,
    force_rehash: bool,
    output_file: str = "",
) -> tuple[int, dict[str, Any]]:
    """Builds a local signed SHA-256 dataset manifest and emits its summary."""
    _log(f"-> Computing SHA-256 dataset manifest in {data_dir} ...")
    manifest = build_backup_manifest(data_dir, dataset_ids=datasets, force_rehash=force_rehash)
    if output_file:
        out_p = Path(output_file).expanduser()
        out_p.parent.mkdir(parents=True, exist_ok=True)
        out_p.write_text(json.dumps(manifest, indent=2, ensure_ascii=False), encoding="utf-8")
    summary = extract_manifest_summary(manifest)
    if output_file:
        summary["written_manifest_path"] = str(Path(output_file).expanduser())
    return EXIT_OK, summary


def run_backup_cmd(
    data_dir: Path,
    targets: list[BackupTarget],
    datasets: list[str] | None,
    options: tuple[bool, bool],
) -> tuple[int, dict[str, Any]]:
    """Executes deduplicated multi-tier snapshot backup across reachable targets."""
    dry_run, force_rehash = options
    reachable, unreachable = _reachable_targets(targets)
    if not reachable:
        return EXIT_UNAVAILABLE, {
            "status": "unavailable",
            "error": "No configured backup targets are currently reachable.",
            "unreachable_targets": unreachable,
        }

    _log(f"-> Scanning datasets and computing incremental SHA-256 manifest ({data_dir})...")
    manifest = build_backup_manifest(data_dir, dataset_ids=datasets, force_rehash=force_rehash)
    snap_id = str(manifest["snapshot_id"])
    _log(f"-> Snapshot {snap_id}: {manifest['total_files']} files ({manifest['total_bytes']} bytes)")

    results: list[dict[str, Any]] = []
    committed_replicas: list[str] = []
    for tgt in reachable:
        _log(f"-> Pushing snapshot {snap_id} to target '{tgt.name}' (dry_run={dry_run})...")
        res = tgt.push_snapshot(data_dir, manifest, dry_run=dry_run)
        results.append(res)
        if res.get("status") in ("committed", "already_committed"):
            committed_replicas.append(tgt.name)

    summary = extract_manifest_summary(manifest)
    if committed_replicas and not dry_run:
        save_latest_backup_summary_cache(summary, committed_replicas)

    return EXIT_OK, {
        "status": "dry_run" if dry_run else "ok",
        "snapshot": summary,
        "replicas_committed": committed_replicas,
        "unreachable_targets": unreachable,
        "target_results": results,
    }


def run_snapshots_cmd(targets: list[BackupTarget]) -> tuple[int, dict[str, Any]]:
    """Lists committed snapshots across all reachable storage tiers."""
    reachable, unreachable = _reachable_targets(targets)
    if not reachable:
        return EXIT_UNAVAILABLE, {"status": "unavailable", "unreachable_targets": unreachable, "targets": []}
    reports = [tgt.list_snapshots() for tgt in reachable]
    return EXIT_OK, {"status": "ok", "unreachable_targets": unreachable, "targets": reports}


def run_verify_cmd(
    targets: list[BackupTarget],
    snapshot_id: str,
    check_hashes: bool,
) -> tuple[int, dict[str, Any]]:
    """Verifies cryptographic manifest signatures and remote blob/pack sizes."""
    reachable, unreachable = _reachable_targets(targets)
    if not reachable:
        return EXIT_UNAVAILABLE, {"status": "unavailable", "unreachable_targets": unreachable, "results": []}
    results = [tgt.verify_snapshot(snapshot_id, check_hashes=check_hashes) for tgt in reachable]
    all_ok = all(r.get("status") == "ok" for r in results)
    return (
        EXIT_OK if all_ok else EXIT_STALE_OR_DRIFT,
        {
            "status": "ok" if all_ok else "failed",
            "snapshot_id": snapshot_id,
            "unreachable_targets": unreachable,
            "results": results,
        },
    )


def _check_replica_covers_local(
    tgt: BackupTarget,
    snapshot_id: str,
    local_files: list[dict[str, Any]],
) -> tuple[bool, str, str]:
    """Verifies that target snapshot is healthy and contains exact SHA-256 parity for all local files."""
    ver = tgt.verify_snapshot(snapshot_id, check_hashes=False)
    if ver.get("status") != "ok":
        return False, "", f"{tgt.name}:verify_failed:{ver.get('errors')}"
    resolved_id = str(ver["snapshot_id"])
    remote_manifest = tgt.fetch_manifest(resolved_id)
    if remote_manifest is None:
        return False, resolved_id, f"{tgt.name}:manifest_missing"
    remote_index = {
        str(r["rel_path"]): (str(r["sha256"]), int(r["size_bytes"])) for r in remote_manifest.get("files", [])
    }
    for lrec in local_files:
        rel = str(lrec["rel_path"])
        pair = remote_index.get(rel)
        if pair is None:
            return False, resolved_id, f"{tgt.name}:unbacked_file:{rel}"
        if pair != (str(lrec["sha256"]), int(lrec["size_bytes"])):
            return False, resolved_id, f"{tgt.name}:hash_drift:{rel}"
    return True, resolved_id, "verified"


def _delete_evicted_dataset_files(data_dir: Path, dataset_ids: list[str], files: list[dict[str, Any]]) -> int:
    """Deletes verified local files and prunes empty child subdirectories."""
    freed = 0
    for rec in files:
        fpath = data_dir / str(rec["rel_path"])
        if fpath.is_file() and not fpath.is_symlink():
            freed += fpath.stat().st_size
            fpath.unlink()
    for ds_id in dataset_ids:
        rel_sub, is_single = DATASET_SPEC_BY_ID[ds_id]
        if is_single:
            continue
        root_sub = data_dir / rel_sub
        if not root_sub.is_dir():
            continue
        for dirpath, dirnames, filenames in os.walk(root_sub, topdown=False):
            p = Path(dirpath)
            if p != root_sub and not dirnames and not filenames:
                p.rmdir()
    return freed


def _record_eviction_tombstone(
    data_dir: Path,
    selected: list[str],
    live_manifest: dict[str, Any],
    evict_meta: tuple[str, list[str]],
) -> None:
    """Persists dataset eviction pointers in data/.evicted_manifest.json before deletion."""
    resolved_snap_id, verified_replicas = evict_meta
    ev_state = load_evicted_manifest(data_dir)
    now_iso = datetime.now(UTC).isoformat(timespec="seconds")
    for ds_id in selected:
        ds_info = live_manifest["datasets"][ds_id]
        if ds_info.get("present"):
            ev_state["evicted_datasets"][ds_id] = {
                "snapshot_id": resolved_snap_id,
                "evicted_at_iso": now_iso,
                "verified_replicas": verified_replicas,
                "file_count": ds_info["file_count"],
                "total_bytes": ds_info["total_bytes"],
                "tree_sha256": ds_info["tree_sha256"],
            }
    save_evicted_manifest(data_dir, ev_state)


def run_evict_cmd(
    data_dir: Path,
    targets: list[BackupTarget],
    datasets: list[str],
    evict_opts: tuple[str, int, bool],
) -> tuple[int, dict[str, Any]]:
    """Evicts local datasets ONLY after proving live SHA-256 parity on >= min_replicas."""
    snapshot_id, min_replicas, apply_delete = evict_opts
    selected = normalize_dataset_ids(datasets)
    for ds_id in selected:
        rel_sub, _ = DATASET_SPEC_BY_ID[ds_id]
        if (data_dir / rel_sub).is_symlink():
            return EXIT_USAGE_ERROR, {
                "status": "blocked_symlink",
                "error": f"Refusing to evict worktree symlink '{rel_sub}'. Run on parent repository if intended.",
            }

    _log(f"-> Recomputing live disk SHA-256 digests before eviction check ({', '.join(selected)})...")
    live_manifest = build_backup_manifest(data_dir, dataset_ids=selected, force_rehash=True)
    local_files: list[dict[str, Any]] = list(live_manifest.get("files", []))
    if not local_files:
        return EXIT_OK, {"status": "noop_already_empty", "datasets": selected, "freed_bytes": 0}

    reachable, unreachable = _reachable_targets(targets)
    verified_replicas: list[str] = []
    replica_diagnostics: dict[str, str] = {}
    resolved_snap_id = snapshot_id

    for tgt in reachable:
        ok, snap_used, detail = _check_replica_covers_local(tgt, snapshot_id, local_files)
        replica_diagnostics[tgt.name] = detail
        if ok:
            verified_replicas.append(tgt.name)
            if resolved_snap_id in ("", "latest"):
                resolved_snap_id = snap_used

    if len(verified_replicas) < min_replicas:
        return EXIT_STALE_OR_DRIFT, {
            "status": "blocked_unverified_replica",
            "required_replicas": min_replicas,
            "verified_replicas": verified_replicas,
            "unreachable_targets": unreachable,
            "diagnostics": replica_diagnostics,
            "freed_bytes": 0,
        }

    planned_bytes = int(live_manifest["total_bytes"])
    if not apply_delete:
        return EXIT_OK, {
            "status": "dry_run_ready_to_evict",
            "snapshot_id": resolved_snap_id,
            "datasets": selected,
            "verified_replicas": verified_replicas,
            "would_free_files": len(local_files),
            "would_free_bytes": planned_bytes,
            "apply_hint": "Pass --apply to execute local deletion.",
        }

    _record_eviction_tombstone(data_dir, selected, live_manifest, (resolved_snap_id, verified_replicas))
    freed_b = _delete_evicted_dataset_files(data_dir, selected, local_files)

    return EXIT_OK, {
        "status": "evicted",
        "snapshot_id": resolved_snap_id,
        "datasets": selected,
        "verified_replicas": verified_replicas,
        "freed_files": len(local_files),
        "freed_bytes": freed_b,
    }


def _select_restore_paths(
    manifest: dict[str, Any],
    data_dir: Path,
    datasets: list[str] | None,
    explicit_paths: list[str],
) -> tuple[set[str], int]:
    """Determines missing or hash-drifted relative paths requiring download."""
    selected_ds = set(normalize_dataset_ids(datasets)) if datasets or not explicit_paths else set()
    wanted_paths = {p.strip().lstrip("/") for p in explicit_paths if p.strip()}
    needed: set[str] = set()
    already_intact = 0

    for rec in manifest.get("files", []):
        rel = str(rec["rel_path"])
        ds_id = str(rec["dataset"])
        matches_filter = (rel in wanted_paths) if wanted_paths else (ds_id in selected_ds)
        if not matches_filter:
            continue
        local_f = data_dir / rel
        if (
            local_f.is_file()
            and local_f.stat().st_size == int(rec["size_bytes"])
            and compute_file_sha256(local_f) == str(rec["sha256"])
        ):
            already_intact += 1
            continue
        needed.add(rel)
    return needed, already_intact


def _order_targets_fastest_first(targets: list[BackupTarget]) -> list[BackupTarget]:
    """Prioritizes local vaults and Gigabit LAN Synology FTP ahead of WAN o2switch."""

    def _rank(tgt: BackupTarget) -> int:
        if tgt.name.startswith("local:"):
            return 0
        if tgt.name == "synology_nas":
            return 1
        return 2

    return sorted(targets, key=_rank)


def _resolve_pinned_snapshot_id(
    ev_state: dict[str, Any],
    snapshot_id: str,
    datasets: list[str] | None,
) -> str:
    """Resolves pinned snapshot ID from eviction tombstone when restoring a single dataset."""
    if snapshot_id in ("", "latest") and datasets and len(datasets) == 1:
        norm_one = normalize_dataset_ids(datasets)[0]
        pinned = ev_state.get("evicted_datasets", {}).get(norm_one, {}).get("snapshot_id")
        if pinned:
            return str(pinned)
    return snapshot_id


def _find_first_manifest(
    reachable: list[BackupTarget],
    snapshot_id: str,
) -> tuple[BackupTarget | None, dict[str, Any] | None]:
    """Fetches the requested snapshot manifest from the fastest reachable target."""
    for tgt in reachable:
        loaded = tgt.fetch_manifest(snapshot_id)
        if loaded is not None:
            return tgt, loaded
    return None, None


def _clear_restored_tombstones(
    data_dir: Path,
    ev_state: dict[str, Any],
    datasets: list[str] | None,
) -> None:
    """Removes restored dataset entries from data/.evicted_manifest.json."""
    restored_ds = normalize_dataset_ids(datasets)
    changed = False
    for ds_id in restored_ds:
        if ds_id in ev_state.get("evicted_datasets", {}):
            del ev_state["evicted_datasets"][ds_id]
            changed = True
    if changed:
        save_evicted_manifest(data_dir, ev_state)


def run_restore_cmd(
    data_dir: Path,
    targets: list[BackupTarget],
    restore_spec: tuple[list[str] | None, list[str], str, bool],
) -> tuple[int, dict[str, Any]]:
    """Restores datasets or specific files from the fastest reachable verified tier."""
    datasets, explicit_paths, snapshot_id, dry_run = restore_spec
    ev_state = load_evicted_manifest(data_dir)
    effective_snap = _resolve_pinned_snapshot_id(ev_state, snapshot_id, datasets)

    reachable, unreachable = _reachable_targets(_order_targets_fastest_first(targets))
    if not reachable:
        return EXIT_UNAVAILABLE, {"status": "unavailable", "unreachable_targets": unreachable}

    chosen_target, manifest = _find_first_manifest(reachable, effective_snap)
    if chosen_target is None or manifest is None:
        return EXIT_UNAVAILABLE, {
            "status": "manifest_not_found",
            "snapshot_id": effective_snap,
            "unreachable_targets": unreachable,
        }

    needed_paths, intact_count = _select_restore_paths(manifest, data_dir, datasets, explicit_paths)
    if not needed_paths:
        return EXIT_OK, {
            "status": "already_up_to_date",
            "target": chosen_target.name,
            "snapshot_id": manifest["snapshot_id"],
            "intact_files": intact_count,
            "restored_files": 0,
        }

    if dry_run:
        return EXIT_OK, {
            "status": "dry_run",
            "target": chosen_target.name,
            "snapshot_id": manifest["snapshot_id"],
            "intact_files": intact_count,
            "would_restore_files": len(needed_paths),
        }

    _log(f"-> Restoring {len(needed_paths)} files from '{chosen_target.name}' (snapshot {manifest['snapshot_id']})...")
    res = chosen_target.restore_files(data_dir, manifest, needed_paths)
    if not explicit_paths:
        _clear_restored_tombstones(data_dir, ev_state, datasets)

    return EXIT_OK, {"status": "restored", **res, "intact_files_skipped": intact_count}


def _build_parser() -> argparse.ArgumentParser:
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--repo", default="", help="Optional repository path override.")
    common.add_argument("--compact", action="store_true", help="Emit single-line JSON.")
    common.add_argument(
        "--target",
        action="append",
        dest="targets",
        default=None,
        help="Target tier (o2switch, synology_nas, local:<dir>). Repeatable; defaults to o2switch + synology_nas.",
    )

    parser = argparse.ArgumentParser(
        prog="backup_cli.py",
        description="Multi-tier SHA-256 dataset snapshot, eviction, and restore CLI.",
        parents=[common],
    )
    sub = parser.add_subparsers(dest="command", required=True)

    p_man = sub.add_parser("manifest", parents=[common], help="Compute local SHA-256 dataset manifest.")
    p_man.add_argument("--dataset", action="append", dest="datasets", default=None)
    p_man.add_argument("--force-rehash", action="store_true")
    p_man.add_argument("--output", default="")

    p_bak = sub.add_parser("backup", parents=[common], help="Create versioned snapshot on configured tiers.")
    p_bak.add_argument("--dataset", action="append", dest="datasets", default=None)
    p_bak.add_argument("--dry-run", action="store_true")
    p_bak.add_argument("--force-rehash", action="store_true")

    sub.add_parser("snapshots", parents=[common], help="List committed snapshots across tiers.")

    p_ver = sub.add_parser("verify", parents=[common], help="Verify remote snapshot manifest & pack integrity.")
    p_ver.add_argument("--snapshot", default="latest")
    p_ver.add_argument("--check-hashes", action="store_true")

    p_evi = sub.add_parser("evict", parents=[common], help="Safely evict local datasets after remote SHA-256 check.")
    p_evi.add_argument("datasets", nargs="+", help="Dataset names to evict (e.g. catalog_dbahash snippets).")
    p_evi.add_argument("--snapshot", default="latest")
    p_evi.add_argument("--min-replicas", type=int, default=DEFAULT_MIN_EVICT_REPLICAS)
    p_evi.add_argument("--apply", action="store_true", help="Execute deletion (default is dry-run).")

    p_res = sub.add_parser("restore", parents=[common], help="Restore datasets or files with SHA-256 verification.")
    p_res.add_argument("datasets", nargs="*", default=None, help="Datasets to restore (default: all).")
    p_res.add_argument("--snapshot", default="latest")
    p_res.add_argument("--path", action="append", dest="paths", default=[])
    p_res.add_argument("--dry-run", action="store_true")

    return parser


def main(argv: list[str] | None = None) -> int:
    """CLI entry point."""
    args = _build_parser().parse_args(argv)
    start_dir = Path(args.repo).resolve() if args.repo else None
    ws = resolve_workspace_context(start_dir)
    data_dir = ws.effective_data_dir

    try:
        if args.command == "manifest":
            code, payload = run_manifest_cmd(data_dir, args.datasets, args.force_rehash, args.output)
        else:
            targets = build_configured_targets(ws, args.targets)
            if args.command == "backup":
                code, payload = run_backup_cmd(data_dir, targets, args.datasets, (args.dry_run, args.force_rehash))
            elif args.command == "snapshots":
                code, payload = run_snapshots_cmd(targets)
            elif args.command == "verify":
                code, payload = run_verify_cmd(targets, args.snapshot, args.check_hashes)
            elif args.command == "evict":
                code, payload = run_evict_cmd(
                    ws.active_repo / "data",
                    targets,
                    args.datasets,
                    (args.snapshot, args.min_replicas, args.apply),
                )
            else:
                code, payload = run_restore_cmd(
                    data_dir,
                    targets,
                    (args.datasets, args.paths, args.snapshot, args.dry_run),
                )
    except ValueError as exc:
        code, payload = EXIT_USAGE_ERROR, {"status": "usage_error", "error": str(exc)}

    out_str = (
        json.dumps(payload, ensure_ascii=False) if args.compact else json.dumps(payload, indent=2, ensure_ascii=False)
    )
    print(out_str)
    return code


if __name__ == "__main__":
    sys.exit(main())
