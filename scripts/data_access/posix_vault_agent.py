#!/usr/bin/env python3
"""Stdlib-only POSIX hardlink snapshot vault agent (runs locally and on o2switch)."""

from __future__ import annotations

import argparse
import hashlib
import hmac
import json
import os
import shutil
import sys
from pathlib import Path
from typing import Any

HASH_READ_CHUNK_BYTES: int = 1024 * 1024
SCHEMA_VERSION: int = 1


def _sha256_file(path: Path) -> str:
    """Computes hex SHA-256 of a file using stdlib C-accelerated I/O."""
    with path.open("rb") as fh:
        if hasattr(hashlib, "file_digest"):
            return hashlib.file_digest(fh, "sha256").hexdigest()
        digest = hashlib.sha256()
        while True:
            chunk = fh.read(HASH_READ_CHUNK_BYTES)
            if not chunk:
                break
            digest.update(chunk)
        return digest.hexdigest()


def _verify_manifest_sig(manifest: dict[str, Any]) -> bool:
    """Verifies canonical SHA-256 signature of a backup manifest."""
    expected = str(manifest.get("manifest_body_sha256", ""))
    if not expected:
        return False
    unsigned = {k: v for k, v in manifest.items() if k != "manifest_body_sha256"}
    canonical = json.dumps(unsigned, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    actual = hashlib.sha256(canonical.encode("utf-8")).hexdigest()
    return hmac.compare_digest(expected, actual)


def _atomic_write_json(dest: Path, payload: dict[str, Any], compact: bool = False) -> None:
    """Atomically writes JSON via a PID-suffixed sibling file."""
    dest.parent.mkdir(parents=True, exist_ok=True)
    tmp = dest.with_name(f".{dest.name}.tmp.{os.getpid()}")
    text = (
        json.dumps(payload, separators=(",", ":"), ensure_ascii=False)
        if compact
        else json.dumps(payload, indent=2, ensure_ascii=False)
    )
    tmp.write_text(text, encoding="utf-8")
    tmp.replace(dest)


def _build_summary(manifest: dict[str, Any]) -> dict[str, Any]:
    """Extracts lightweight summary metadata from a full manifest."""
    return {
        "schema_version": manifest.get("schema_version", SCHEMA_VERSION),
        "snapshot_id": manifest["snapshot_id"],
        "created_at_iso": manifest["created_at_iso"],
        "created_at_epoch": manifest.get("created_at_epoch", 0),
        "git_commit": manifest.get("git_commit", ""),
        "source_host": manifest.get("source_host", ""),
        "total_files": manifest.get("total_files", 0),
        "total_bytes": manifest.get("total_bytes", 0),
        "manifest_body_sha256": manifest["manifest_body_sha256"],
        "datasets": manifest.get("datasets", {}),
    }


def _load_manifest_checked(manifest_path: Path) -> dict[str, Any]:
    """Loads and cryptographically validates a manifest file."""
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if not _verify_manifest_sig(manifest):
        raise ValueError(f"Manifest signature verification failed: {manifest_path}")
    return manifest


def _resolve_prev_snapshot(vault_dir: Path) -> tuple[Path | None, dict[str, str]]:
    """Resolves previous committed snapshot data directory and its rel_path -> sha256 map."""
    latest_file = vault_dir / "latest.json"
    if not latest_file.is_file():
        return None, {}
    try:
        info = json.loads(latest_file.read_text(encoding="utf-8"))
        prev_id = str(info.get("snapshot_id", ""))
        cand = vault_dir / "snapshots" / prev_id / "data"
        mpath = vault_dir / "manifests" / f"{prev_id}.json"
        if not prev_id or not cand.is_dir() or not mpath.is_file():
            return None, {}
        prev_manifest = json.loads(mpath.read_text(encoding="utf-8"))
        prev_hashes = {str(r["rel_path"]): str(r["sha256"]) for r in prev_manifest.get("files", [])}
        return cand, prev_hashes
    except (OSError, json.JSONDecodeError):
        return None, {}


def _try_hardlink_existing(
    rec: dict[str, Any],
    dst: Path,
    prev_ctx: tuple[Path | None, dict[str, str]],
    seed_data: Path | None,
) -> str:
    """Attempts zero-copy hardlink from SHA-256 verified previous snapshot or live seed dir."""
    rel = str(rec["rel_path"])
    size_b = int(rec["size_bytes"])
    exp_sha = str(rec["sha256"])
    prev_data, prev_hashes = prev_ctx

    if prev_data is not None and prev_hashes.get(rel) == exp_sha:
        prev_src = prev_data / rel
        if prev_src.is_file() and prev_src.stat().st_size == size_b:
            dst.parent.mkdir(parents=True, exist_ok=True)
            os.link(prev_src, dst)
            return "prev"

    if seed_data is not None:
        seed_src = seed_data / rel
        if seed_src.is_file() and seed_src.stat().st_size == size_b and _sha256_file(seed_src) == exp_sha:
            dst.parent.mkdir(parents=True, exist_ok=True)
            os.link(seed_src, dst)
            return "seed"

    return "missing"


def stage_snapshot(vault_dir: Path, manifest_path: Path, seed_data_dir: Path | None) -> dict[str, Any]:
    """Pre-links unchanged files into staging dir and lists missing files for delta rsync."""
    manifest = _load_manifest_checked(manifest_path)
    snap_id = str(manifest["snapshot_id"])
    final_dir = vault_dir / "snapshots" / snap_id
    if final_dir.exists():
        return {
            "status": "already_committed",
            "snapshot_id": snap_id,
            "linked_from_prev": 0,
            "linked_from_seed": 0,
            "missing_count": 0,
            "missing_bytes": 0,
            "missing_files": [],
        }

    staging_dir = vault_dir / "snapshots" / f".staging_{snap_id}"
    if staging_dir.exists():
        shutil.rmtree(staging_dir)
    staging_data = staging_dir / "data"
    staging_data.mkdir(parents=True, exist_ok=True)

    prev_ctx = _resolve_prev_snapshot(vault_dir)
    linked_prev = 0
    linked_seed = 0
    missing_files: list[str] = []
    missing_bytes = 0

    for rec in manifest.get("files", []):
        rel = str(rec["rel_path"])
        dst = staging_data / rel
        outcome = _try_hardlink_existing(rec, dst, prev_ctx, seed_data_dir)
        if outcome == "prev":
            linked_prev += 1
        elif outcome == "seed":
            linked_seed += 1
        else:
            missing_files.append(rel)
            missing_bytes += int(rec["size_bytes"])

    (staging_dir / "missing_files.json").write_text(json.dumps(missing_files), encoding="utf-8")
    return {
        "status": "staged",
        "snapshot_id": snap_id,
        "staging_data_dir": str(staging_data),
        "linked_from_prev": linked_prev,
        "linked_from_seed": linked_seed,
        "missing_count": len(missing_files),
        "missing_bytes": missing_bytes,
        "missing_files": missing_files,
    }


def _verify_staged_files(staging_data: Path, files: list[dict[str, Any]], hash_all_or_set: set[str] | None) -> int:
    """Verifies presence, byte size, and SHA-256 of newly synced files in staging_data."""
    verified_new = 0
    for rec in files:
        rel = str(rec["rel_path"])
        fpath = staging_data / rel
        if not fpath.is_file():
            raise ValueError(f"Missing staged file: {rel}")
        actual_size = fpath.stat().st_size
        exp_size = int(rec["size_bytes"])
        if actual_size != exp_size:
            raise ValueError(f"Size mismatch for {rel}: expected {exp_size}, got {actual_size}")
        if hash_all_or_set is None or rel in hash_all_or_set:
            actual_sha = _sha256_file(fpath)
            if actual_sha != str(rec["sha256"]):
                raise ValueError(f"SHA-256 mismatch for {rel}: expected {rec['sha256']}, got {actual_sha}")
            verified_new += 1
    return verified_new


def commit_snapshot(vault_dir: Path, manifest_path: Path) -> dict[str, Any]:
    """Verifies SHA-256 of newly synced files and atomically publishes snapshot + manifest."""
    manifest = _load_manifest_checked(manifest_path)
    snap_id = str(manifest["snapshot_id"])
    final_dir = vault_dir / "snapshots" / snap_id
    staging_dir = vault_dir / "snapshots" / f".staging_{snap_id}"

    if final_dir.is_dir() and (vault_dir / "manifests" / f"{snap_id}.json").is_file():
        return {"status": "already_committed", "snapshot_id": snap_id, "verified_new_files": 0}
    if not staging_dir.is_dir():
        raise FileNotFoundError(f"Staging directory missing for snapshot {snap_id}: {staging_dir}")

    missing_json = staging_dir / "missing_files.json"
    must_hash = set(json.loads(missing_json.read_text(encoding="utf-8"))) if missing_json.is_file() else None
    verified_new = _verify_staged_files(staging_dir / "data", manifest.get("files", []), must_hash)

    missing_json.unlink(missing_ok=True)
    os.replace(staging_dir, final_dir)

    manifests_dir = vault_dir / "manifests"
    _atomic_write_json(manifests_dir / f"{snap_id}.json", manifest, compact=True)
    summary = _build_summary(manifest)
    _atomic_write_json(manifests_dir / f"{snap_id}.summary.json", summary, compact=False)
    _atomic_write_json(
        vault_dir / "latest.json",
        {
            "snapshot_id": snap_id,
            "created_at_iso": manifest["created_at_iso"],
            "git_commit": manifest.get("git_commit", ""),
            "total_files": manifest.get("total_files", 0),
            "total_bytes": manifest.get("total_bytes", 0),
            "manifest_body_sha256": manifest["manifest_body_sha256"],
        },
    )

    latest_link = vault_dir / "latest"
    tmp_link = vault_dir / f".latest.tmp.{os.getpid()}"
    if tmp_link.is_symlink() or tmp_link.exists():
        tmp_link.unlink()
    tmp_link.symlink_to(Path("snapshots") / snap_id)
    os.replace(tmp_link, latest_link)

    return {
        "status": "committed",
        "snapshot_id": snap_id,
        "verified_new_files": verified_new,
        "total_files": manifest.get("total_files", 0),
    }


def verify_snapshot(vault_dir: Path, snapshot_id: str, check_hashes: bool = False) -> dict[str, Any]:
    """Verifies manifest signature, file presence, byte sizes, and optional SHA-256 digests."""
    resolved_id = snapshot_id
    if resolved_id in ("", "latest"):
        latest_file = vault_dir / "latest.json"
        if not latest_file.is_file():
            return {"status": "empty", "snapshot_id": "", "checked_files": 0, "errors": ["No snapshots found"]}
        resolved_id = str(json.loads(latest_file.read_text(encoding="utf-8")).get("snapshot_id", ""))

    mpath = vault_dir / "manifests" / f"{resolved_id}.json"
    if not mpath.is_file():
        return {"status": "missing", "snapshot_id": resolved_id, "checked_files": 0, "errors": [f"Missing {mpath}"]}

    try:
        manifest = _load_manifest_checked(mpath)
    except ValueError as exc:
        return {"status": "corrupt", "snapshot_id": resolved_id, "checked_files": 0, "errors": [str(exc)]}

    snap_data = vault_dir / "snapshots" / resolved_id / "data"
    errors: list[str] = []
    checked = 0
    for rec in manifest.get("files", []):
        rel = str(rec["rel_path"])
        fpath = snap_data / rel
        if not fpath.is_file():
            errors.append(f"missing:{rel}")
            continue
        if fpath.stat().st_size != int(rec["size_bytes"]):
            errors.append(f"size_mismatch:{rel}")
            continue
        if check_hashes and _sha256_file(fpath) != str(rec["sha256"]):
            errors.append(f"sha256_mismatch:{rel}")
            continue
        checked += 1

    return {
        "status": "ok" if not errors else "corrupt",
        "snapshot_id": resolved_id,
        "manifest_body_sha256": manifest["manifest_body_sha256"],
        "checked_files": checked,
        "hashes_verified": check_hashes,
        "errors": errors,
    }


def list_snapshots(vault_dir: Path) -> dict[str, Any]:
    """Lists all committed snapshot summaries sorted newest-first."""
    latest_id = ""
    latest_file = vault_dir / "latest.json"
    if latest_file.is_file():
        try:
            latest_id = str(json.loads(latest_file.read_text(encoding="utf-8")).get("snapshot_id", ""))
        except (OSError, json.JSONDecodeError):
            latest_id = ""

    manifests_dir = vault_dir / "manifests"
    summaries: list[dict[str, Any]] = []
    if manifests_dir.is_dir():
        for spath in sorted(manifests_dir.glob("*.summary.json"), reverse=True):
            try:
                summaries.append(json.loads(spath.read_text(encoding="utf-8")))
            except (OSError, json.JSONDecodeError):
                continue

    return {
        "vault_dir": str(vault_dir),
        "latest_snapshot_id": latest_id,
        "snapshot_count": len(summaries),
        "snapshots": summaries,
    }


def _build_cli() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="POSIX snapshot vault agent.")
    sub = parser.add_subparsers(dest="action", required=True)

    p_stage = sub.add_parser("stage")
    p_stage.add_argument("--vault-dir", required=True)
    p_stage.add_argument("--manifest", required=True)
    p_stage.add_argument("--seed-data-dir", default="")

    p_commit = sub.add_parser("commit")
    p_commit.add_argument("--vault-dir", required=True)
    p_commit.add_argument("--manifest", required=True)

    p_verify = sub.add_parser("verify")
    p_verify.add_argument("--vault-dir", required=True)
    p_verify.add_argument("--snapshot-id", default="latest")
    p_verify.add_argument("--check-hashes", action="store_true")

    p_list = sub.add_parser("list")
    p_list.add_argument("--vault-dir", required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    """CLI entry point emitting deterministic JSON."""
    args = _build_cli().parse_args(argv)
    vault = Path(args.vault_dir).expanduser().resolve()
    try:
        if args.action == "stage":
            seed = Path(args.seed_data_dir).expanduser().resolve() if args.seed_data_dir else None
            res = stage_snapshot(vault, Path(args.manifest).expanduser().resolve(), seed)
        elif args.action == "commit":
            res = commit_snapshot(vault, Path(args.manifest).expanduser().resolve())
        elif args.action == "verify":
            res = verify_snapshot(vault, args.snapshot_id, args.check_hashes)
            print(json.dumps(res, ensure_ascii=False))
            return 0 if res["status"] == "ok" else 4
        else:
            res = list_snapshots(vault)
        print(json.dumps(res, ensure_ascii=False))
        return 0
    except Exception as exc:
        print(json.dumps({"status": "error", "error": str(exc)}, ensure_ascii=False))
        return 1


if __name__ == "__main__":
    sys.exit(main())
