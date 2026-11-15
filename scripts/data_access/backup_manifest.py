"""Deterministic SHA-256 dataset manifest builder, hash cache, and streaming TAR packer."""

from __future__ import annotations

import hashlib
import hmac
import io
import json
import os
import socket
import struct
import tarfile
import time
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from scripts.data_access.constants import (
    BACKUP_DATASET_SPECS,
    BACKUP_EXCLUDED_DIR_NAMES,
    BACKUP_HASH_CACHE_FILENAME,
    BACKUP_HASH_CACHE_VERSION,
    BACKUP_HASH_READ_CHUNK_BYTES,
    BACKUP_MANIFEST_SCHEMA_VERSION,
    BACKUP_TRANSIENT_SUFFIXES,
    CACHE_DIR_MODE,
    CACHE_DIR_REL,
    CACHE_FILE_MODE,
    DBAHASH_HEADER_PREFIX_BYTES,
    DBAHASH_HEADER_PREFIX_FORMAT,
    DBAHASH_MAGIC_PREFIX,
    DBPHASH_HEADER_PREFIX_BYTES,
    DBPHASH_HEADER_PREFIX_FORMAT,
    DBPHASH_MAGIC_PREFIX,
    EVICTED_MANIFEST_FILENAME,
    GIT_CMD_TIMEOUT_SECS,
    GIT_SHORT_SHA_LEN,
    LATEST_BACKUP_CACHE_FILENAME,
    PACK_TREE_HASH_PREFIX_LEN,
    TAR_BLOCK_BYTES,
    TAR_DEFAULT_FILE_MODE,
    TAR_EOF_BLOCKS_COUNT,
)
from scripts.data_access.topology import run_bounded_cmd

DATASET_SPEC_BY_ID: dict[str, tuple[str, bool]] = {
    ds_id: (rel_subpath, is_single) for ds_id, rel_subpath, is_single in BACKUP_DATASET_SPECS
}

DATASET_ALIASES: dict[str, str] = {
    "dbphash": "catalog_dbphash",
    "catalog.dbphash": "catalog_dbphash",
    "catalog": "catalog_dbphash",
    "dbahash": "catalog_dbahash",
    "catalog.dbahash": "catalog_dbahash",
    "phash": "phash_corpus",
    "__phash__": "phash_corpus",
    "ahash": "ahash_corpus",
    "__ahash__": "ahash_corpus",
    "srt": "srt_store",
    "couplings": "srt_store",
    "couplings.json": "srt_store",
    "quarantine": "quarantine_phash",
}


def normalize_dataset_ids(raw_names: list[str] | tuple[str, ...] | None) -> list[str]:
    """Resolves user dataset names or aliases into canonical ordered dataset IDs."""
    if not raw_names:
        return [ds_id for ds_id, _, _ in BACKUP_DATASET_SPECS]
    requested: set[str] = set()
    for raw in raw_names:
        key = raw.strip()
        low = key.lower()
        if low in ("all", "*"):
            return [ds_id for ds_id, _, _ in BACKUP_DATASET_SPECS]
        resolved = DATASET_ALIASES.get(low, key)
        if resolved not in DATASET_SPEC_BY_ID:
            valid = ", ".join(DATASET_SPEC_BY_ID)
            raise ValueError(f"Unknown dataset '{raw}'. Valid datasets: {valid}")
        requested.add(resolved)
    return [ds_id for ds_id, _, _ in BACKUP_DATASET_SPECS if ds_id in requested]


def get_backup_hash_cache_path() -> Path:
    """Returns the user-local incremental SHA-256 cache file path."""
    override = os.environ.get("DBSRT_BACKUP_HASH_CACHE")
    if override:
        return Path(override)
    return Path.home().joinpath(*CACHE_DIR_REL, BACKUP_HASH_CACHE_FILENAME)


def load_hash_cache(cache_path: Path) -> dict[str, dict[str, Any]]:
    """Loads cached (size, mtime_ns) -> sha256 entries, returning empty dict on miss."""
    if not cache_path.is_file():
        return {}
    try:
        payload = json.loads(cache_path.read_text(encoding="utf-8"))
        if payload.get("version") != BACKUP_HASH_CACHE_VERSION:
            return {}
        entries = payload.get("entries", {})
        return entries if isinstance(entries, dict) else {}
    except (OSError, json.JSONDecodeError):
        return {}


def save_hash_cache(cache_path: Path, entries: dict[str, dict[str, Any]]) -> None:
    """Atomically persists the incremental SHA-256 hash cache."""
    cache_path.parent.mkdir(parents=True, exist_ok=True, mode=CACHE_DIR_MODE)
    tmp_path = cache_path.with_name(f".{cache_path.name}.tmp.{os.getpid()}")
    payload = {
        "version": BACKUP_HASH_CACHE_VERSION,
        "updated_at_epoch": int(time.time()),
        "entries": entries,
    }
    tmp_path.write_text(json.dumps(payload, separators=(",", ":")), encoding="utf-8")
    os.chmod(tmp_path, CACHE_FILE_MODE)
    tmp_path.replace(cache_path)


def compute_file_sha256(path: Path) -> str:
    """Computes the hex SHA-256 digest of a file using C-accelerated streaming I/O."""
    with path.open("rb") as fh:
        if hasattr(hashlib, "file_digest"):
            return hashlib.file_digest(fh, "sha256").hexdigest()
        digest = hashlib.sha256()
        while True:
            chunk = fh.read(BACKUP_HASH_READ_CHUNK_BYTES)
            if not chunk:
                break
            digest.update(chunk)
        return digest.hexdigest()


def _is_transient_or_excluded(path: Path) -> bool:
    """Returns True for temporary swap/staging files that must not enter snapshots."""
    name = path.name
    if name == EVICTED_MANIFEST_FILENAME or name.startswith(".tmp."):
        return True
    low = name.lower()
    if any(low.endswith(sfx) or f"{sfx}." in low for sfx in BACKUP_TRANSIENT_SUFFIXES):
        return True
    return any(part in BACKUP_EXCLUDED_DIR_NAMES for part in path.parts)


def _iter_dataset_files(data_dir: Path, rel_subpath: str, is_single_file: bool) -> list[Path]:
    """Enumerates regular files belonging to a dataset sorted deterministically."""
    target = data_dir / rel_subpath
    if not target.exists():
        return []
    if is_single_file:
        return [target] if target.is_file() else []
    if not target.is_dir():
        return []
    collected: list[Path] = []
    for root, dirs, files in os.walk(target, followlinks=True):
        dirs[:] = sorted(d for d in dirs if d not in BACKUP_EXCLUDED_DIR_NAMES)
        root_path = Path(root)
        for fname in sorted(files):
            fpath = root_path / fname
            if not _is_transient_or_excluded(fpath) and fpath.is_file():
                collected.append(fpath)
    collected.sort(key=lambda p: p.relative_to(data_dir).as_posix())
    return collected


def _inspect_binary_header_count(path: Path, magic: bytes, fmt: str, nbytes: int, idx: int) -> int | None:
    """Reads O(1) media count from a packed .dbphash or .dbahash header."""
    if not path.is_file():
        return None
    try:
        with path.open("rb") as fh:
            raw = fh.read(nbytes)
        if len(raw) < nbytes or not raw.startswith(magic):
            return None
        unpacked = struct.unpack(fmt, raw)
        return int(unpacked[idx])
    except (OSError, struct.error):
        return None


def _count_couplings_entries(cpath: Path) -> dict[str, Any]:
    """Counts certified couplings in couplings.json without raising on malformed files."""
    if not cpath.is_file():
        return {}
    try:
        loaded = json.loads(cpath.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}
    if isinstance(loaded, list):
        return {"couplings_count": len(loaded)}
    if isinstance(loaded, dict):
        entries = loaded.get("couplings", loaded)
        if isinstance(entries, dict | list):
            return {"couplings_count": len(entries)}
    return {}


def _extract_domain_metadata(dataset_id: str, data_dir: Path) -> dict[str, Any]:
    """Extracts lightweight domain invariants (media/coupling counts) for auditability."""
    if dataset_id == "catalog_dbphash":
        cnt = _inspect_binary_header_count(
            data_dir / "catalog.dbphash",
            DBPHASH_MAGIC_PREFIX,
            DBPHASH_HEADER_PREFIX_FORMAT,
            DBPHASH_HEADER_PREFIX_BYTES,
            3,
        )
        return {"format": "dbphash_v2", "media_count": cnt} if cnt is not None else {}
    if dataset_id == "catalog_dbahash":
        cnt = _inspect_binary_header_count(
            data_dir / "catalog.dbahash",
            DBAHASH_MAGIC_PREFIX,
            DBAHASH_HEADER_PREFIX_FORMAT,
            DBAHASH_HEADER_PREFIX_BYTES,
            4,
        )
        return {"format": "dbahash_v1", "media_count": cnt} if cnt is not None else {}
    if dataset_id == "srt_store":
        return _count_couplings_entries(data_dir / "SRT" / "couplings.json")
    return {}


def compute_tree_sha256(file_records: list[dict[str, Any]]) -> str:
    """Computes a canonical Merkle-like SHA-256 over sorted (rel_path, sha256, size_bytes)."""
    hasher = hashlib.sha256()
    for rec in sorted(file_records, key=lambda r: str(r["rel_path"])):
        line = f"{rec['rel_path']}\x00{rec['sha256']}\x00{rec['size_bytes']}\n"
        hasher.update(line.encode("utf-8"))
    return hasher.hexdigest()


def compute_manifest_body_sha256(manifest: dict[str, Any]) -> str:
    """Computes canonical SHA-256 signature of a manifest excluding 'manifest_body_sha256'."""
    unsigned = {k: v for k, v in manifest.items() if k != "manifest_body_sha256"}
    canonical = json.dumps(unsigned, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def verify_manifest_signature(manifest: dict[str, Any]) -> bool:
    """Cryptographically verifies manifest integrity against tampering or truncation."""
    expected = str(manifest.get("manifest_body_sha256", ""))
    if not expected:
        return False
    actual = compute_manifest_body_sha256(manifest)
    return hmac.compare_digest(expected, actual)


def resolve_git_short_sha(repo_dir: Path) -> str:
    """Returns the short 7-hex Git commit SHA or 'nogit' outside repositories."""
    rc, out, _ = run_bounded_cmd(
        ["git", "rev-parse", f"--short={GIT_SHORT_SHA_LEN}", "HEAD"], GIT_CMD_TIMEOUT_SECS, cwd=repo_dir
    )
    return out.strip() if rc == 0 and out.strip() else "nogit"


def _hash_or_reuse_file(
    fpath: Path,
    rel_posix: str,
    dataset_id: str,
    cache_entries: dict[str, dict[str, Any]],
    force_rehash: bool,
) -> tuple[dict[str, Any], bool]:
    """Hashes a file or reuses cached SHA-256 when (size_bytes, mtime_ns) match."""
    st = fpath.stat()
    size_b = int(st.st_size)
    mtime_ns = int(st.st_mtime_ns)
    mtime_sec = int(st.st_mtime)
    cached = None if force_rehash else cache_entries.get(rel_posix)
    if cached and cached.get("size") == size_b and cached.get("mtime_ns") == mtime_ns and cached.get("sha256"):
        sha = str(cached["sha256"])
        rehashed = False
    else:
        sha = compute_file_sha256(fpath)
        cache_entries[rel_posix] = {"size": size_b, "mtime_ns": mtime_ns, "sha256": sha}
        rehashed = True
    return (
        {
            "rel_path": rel_posix,
            "dataset": dataset_id,
            "size_bytes": size_b,
            "mtime_epoch": mtime_sec,
            "sha256": sha,
        },
        rehashed,
    )


def _scan_single_dataset(
    ds_id: str,
    data_dir: Path,
    cache_entries: dict[str, dict[str, Any]],
    force_rehash: bool,
) -> tuple[list[dict[str, Any]], dict[str, Any], bool]:
    """Scans and hashes one dataset, returning records, summary entry, and cache_dirty flag."""
    rel_sub, is_single = DATASET_SPEC_BY_ID[ds_id]
    paths = _iter_dataset_files(data_dir, rel_sub, is_single)
    ds_records: list[dict[str, Any]] = []
    dirty = False
    for fpath in paths:
        rel_posix = fpath.relative_to(data_dir).as_posix()
        rec, was_rehashed = _hash_or_reuse_file(fpath, rel_posix, ds_id, cache_entries, force_rehash)
        dirty = dirty or was_rehashed
        ds_records.append(rec)

    total_b = sum(int(r["size_bytes"]) for r in ds_records)
    tree_sha = compute_tree_sha256(ds_records) if ds_records else ""
    pack_name = f"{ds_id}_{tree_sha[:PACK_TREE_HASH_PREFIX_LEN]}.tar" if tree_sha else ""
    summary = {
        "rel_subpath": rel_sub,
        "is_single_file": is_single,
        "present": bool(ds_records),
        "file_count": len(ds_records),
        "total_bytes": total_b,
        "tree_sha256": tree_sha,
        "pack_filename": pack_name,
        "domain_meta": _extract_domain_metadata(ds_id, data_dir) if ds_records else {},
    }
    return ds_records, summary, dirty


def build_backup_manifest(
    data_dir: Path,
    dataset_ids: list[str] | None = None,
    force_rehash: bool = False,
    git_commit: str = "",
) -> dict[str, Any]:
    """Scans data_dir, computes/reuses SHA-256 digests, and returns a signed v1 manifest."""
    selected = normalize_dataset_ids(dataset_ids)
    cache_path = get_backup_hash_cache_path()
    cache_entries = load_hash_cache(cache_path)
    cache_dirty = False

    now_utc = datetime.now(UTC)
    commit_tag = git_commit or resolve_git_short_sha(data_dir.parent)
    ts_compact = now_utc.strftime("%Y%m%dT%H%M%SZ")
    snapshot_id = f"{ts_compact}_{commit_tag}"

    datasets_summary: dict[str, Any] = {}
    all_files: list[dict[str, Any]] = []

    for ds_id in selected:
        ds_records, ds_sum, dirty = _scan_single_dataset(ds_id, data_dir, cache_entries, force_rehash)
        cache_dirty = cache_dirty or dirty
        all_files.extend(ds_records)
        datasets_summary[ds_id] = ds_sum

    if cache_dirty:
        save_hash_cache(cache_path, cache_entries)

    manifest: dict[str, Any] = {
        "schema_version": BACKUP_MANIFEST_SCHEMA_VERSION,
        "snapshot_id": snapshot_id,
        "created_at_iso": now_utc.isoformat(timespec="seconds"),
        "created_at_epoch": int(now_utc.timestamp()),
        "git_commit": commit_tag,
        "source_host": socket.gethostname(),
        "total_files": len(all_files),
        "total_bytes": sum(int(r["size_bytes"]) for r in all_files),
        "datasets": datasets_summary,
        "files": all_files,
    }
    manifest["manifest_body_sha256"] = compute_manifest_body_sha256(manifest)
    return manifest


def extract_manifest_summary(manifest: dict[str, Any]) -> dict[str, Any]:
    """Produces a compact (<4 KB) snapshot summary excluding per-file records."""
    return {
        "schema_version": manifest.get("schema_version", BACKUP_MANIFEST_SCHEMA_VERSION),
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


def load_evicted_manifest(data_dir: Path) -> dict[str, Any]:
    """Loads local tombstone pointers for intentionally evicted datasets."""
    ev_path = data_dir / EVICTED_MANIFEST_FILENAME
    if not ev_path.is_file():
        return {"version": BACKUP_MANIFEST_SCHEMA_VERSION, "evicted_datasets": {}}
    try:
        data = json.loads(ev_path.read_text(encoding="utf-8"))
        if isinstance(data, dict) and isinstance(data.get("evicted_datasets"), dict):
            return data
    except (OSError, json.JSONDecodeError):
        pass
    return {"version": BACKUP_MANIFEST_SCHEMA_VERSION, "evicted_datasets": {}}


def save_evicted_manifest(data_dir: Path, payload: dict[str, Any]) -> None:
    """Atomically writes or removes data/.evicted_manifest.json."""
    ev_path = data_dir / EVICTED_MANIFEST_FILENAME
    if not payload.get("evicted_datasets"):
        ev_path.unlink(missing_ok=True)
        return
    data_dir.mkdir(parents=True, exist_ok=True)
    tmp = ev_path.with_name(f"{EVICTED_MANIFEST_FILENAME}.tmp.{os.getpid()}")
    tmp.write_text(json.dumps(payload, indent=2, ensure_ascii=False), encoding="utf-8")
    tmp.replace(ev_path)


def get_latest_backup_cache_path() -> Path:
    """Returns path to cached latest committed backup summary."""
    override = os.environ.get("DBSRT_LATEST_BACKUP_CACHE")
    if override:
        return Path(override)
    return Path.home().joinpath(*CACHE_DIR_REL, LATEST_BACKUP_CACHE_FILENAME)


def save_latest_backup_summary_cache(summary: dict[str, Any], replicas: list[str]) -> None:
    """Atomically caches the latest committed snapshot summary for <1ms status checks."""
    cpath = get_latest_backup_cache_path()
    cpath.parent.mkdir(parents=True, exist_ok=True, mode=CACHE_DIR_MODE)
    tmp = cpath.with_name(f".{cpath.name}.tmp.{os.getpid()}")
    payload = {**summary, "verified_replicas": replicas}
    tmp.write_text(json.dumps(payload, indent=2, ensure_ascii=False), encoding="utf-8")
    os.chmod(tmp, CACHE_FILE_MODE)
    tmp.replace(cpath)


def load_latest_backup_summary_cache() -> dict[str, Any] | None:
    """Reads cached latest backup summary if present."""
    cpath = get_latest_backup_cache_path()
    if not cpath.is_file():
        return None
    try:
        data = json.loads(cpath.read_text(encoding="utf-8"))
        return data if isinstance(data, dict) and data.get("snapshot_id") else None
    except (OSError, json.JSONDecodeError):
        return None


def inspect_local_backup_health(data_dir: Path) -> dict[str, Any]:
    """Fast (<2ms) zero-network backup & eviction status summary for dbsrt_data.py status."""
    latest = load_latest_backup_summary_cache()
    ev_state = load_evicted_manifest(data_dir)
    evicted_map = ev_state.get("evicted_datasets", {})

    if latest is None:
        return {
            "status": "NO_LOCAL_BACKUP_RECORD",
            "latest_snapshot_id": None,
            "verified_replicas": [],
            "evicted_datasets": sorted(evicted_map.keys()),
            "hint": "Run 'task data:backup' to create a verified multi-tier snapshot.",
        }

    snap_epoch = int(latest.get("created_at_epoch", 0))
    backed_ds = latest.get("datasets", {})
    drifted_or_unbacked: list[str] = []
    for ds_id, (rel_sub, is_single) in DATASET_SPEC_BY_ID.items():
        if ds_id in evicted_map:
            continue
        p = data_dir / rel_sub
        if not p.exists():
            continue
        prev_present = bool(backed_ds.get(ds_id, {}).get("present"))
        if not prev_present:
            drifted_or_unbacked.append(ds_id)
            continue
        if is_single and int(p.stat().st_mtime) > snap_epoch:
            drifted_or_unbacked.append(ds_id)

    return {
        "status": "DRIFT_DETECTED" if drifted_or_unbacked else "BACKED_UP",
        "latest_snapshot_id": latest.get("snapshot_id"),
        "latest_created_at_iso": latest.get("created_at_iso"),
        "verified_replicas": latest.get("verified_replicas", []),
        "drifted_or_unbacked_datasets": drifted_or_unbacked,
        "evicted_datasets": sorted(evicted_map.keys()),
    }


def build_tar_header_bytes(rel_path: str, size_bytes: int, mtime_epoch: int) -> bytes:
    """Builds a deterministic POSIX PAX tar header for a single regular file."""
    ti = tarfile.TarInfo(name=rel_path)
    ti.size = size_bytes
    ti.mtime = mtime_epoch
    ti.mode = TAR_DEFAULT_FILE_MODE
    ti.type = tarfile.REGTYPE
    ti.uid = 0
    ti.gid = 0
    ti.uname = ""
    ti.gname = ""
    return ti.tobuf(tarfile.PAX_FORMAT, "utf-8", "strict")


class DeterministicTarStreamReader(io.RawIOBase):
    """Zero-temp-disk streaming POSIX PAX tar reader for ftplib.FTP.storbinary()."""

    def __init__(self, data_dir: Path, file_records: list[dict[str, Any]]) -> None:
        super().__init__()
        self._data_dir = data_dir
        self._records = sorted(file_records, key=lambda r: str(r["rel_path"]))
        self._headers: list[bytes] = []
        total = TAR_BLOCK_BYTES * TAR_EOF_BLOCKS_COUNT
        for rec in self._records:
            size_b = int(rec["size_bytes"])
            hdr = build_tar_header_bytes(str(rec["rel_path"]), size_b, int(rec["mtime_epoch"]))
            self._headers.append(hdr)
            pad = (TAR_BLOCK_BYTES - (size_b % TAR_BLOCK_BYTES)) % TAR_BLOCK_BYTES
            total += len(hdr) + size_b + pad
        self.expected_total_bytes: int = total
        self._idx: int = 0
        self._cur_fh: io.BufferedReader | None = None
        self._cur_rem: int = 0
        self._cur_pad: int = 0
        self._eof_sent: bool = False
        self._buf = bytearray()

    def readable(self) -> bool:
        """Marks stream as readable for io.RawIOBase consumers."""
        return True

    def _fill_next_chunk(self) -> bool:
        """Appends the next header, file payload slice, padding, or EOF trailer into _buf."""
        if self._cur_fh is not None:
            if self._cur_rem > 0:
                chunk = self._cur_fh.read(min(BACKUP_HASH_READ_CHUNK_BYTES, self._cur_rem))
                if not chunk:
                    self._cur_fh.close()
                    self._cur_fh = None
                    raise OSError(f"File truncated while streaming tar pack at index {self._idx - 1}")
                self._cur_rem -= len(chunk)
                self._buf.extend(chunk)
                return True
            self._cur_fh.close()
            self._cur_fh = None
            if self._cur_pad > 0:
                self._buf.extend(b"\x00" * self._cur_pad)
                self._cur_pad = 0
                return True

        if self._idx < len(self._records):
            rec = self._records[self._idx]
            hdr = self._headers[self._idx]
            self._idx += 1
            size_b = int(rec["size_bytes"])
            self._buf.extend(hdr)
            if size_b > 0:
                fpath = self._data_dir / str(rec["rel_path"])
                self._cur_fh = fpath.open("rb")
                self._cur_rem = size_b
                self._cur_pad = (TAR_BLOCK_BYTES - (size_b % TAR_BLOCK_BYTES)) % TAR_BLOCK_BYTES
            return True

        if not self._eof_sent:
            self._eof_sent = True
            self._buf.extend(b"\x00" * (TAR_BLOCK_BYTES * TAR_EOF_BLOCKS_COUNT))
            return True
        return False

    def read(self, size: int = -1) -> bytes:
        """Reads up to 'size' bytes from the virtual deterministic tar stream."""
        if size is None or size < 0:
            while self._fill_next_chunk():
                pass
            out = bytes(self._buf)
            self._buf.clear()
            return out
        while len(self._buf) < size:
            if not self._fill_next_chunk():
                break
        out = bytes(self._buf[:size])
        del self._buf[:size]
        return out

    def close(self) -> None:
        """Closes any open underlying file handle."""
        if self._cur_fh is not None:
            self._cur_fh.close()
            self._cur_fh = None
        super().close()
