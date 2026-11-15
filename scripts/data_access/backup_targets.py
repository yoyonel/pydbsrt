"""Multi-tier backup & restore targets: Local POSIX, Cloud o2switch (SSH/rsync), Synology FTP."""

from __future__ import annotations

import ftplib
import io
import json
import os
import shutil
import subprocess
import sys
import tarfile
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol

from scripts.data_access import posix_vault_agent
from scripts.data_access.backup_manifest import (
    DeterministicTarStreamReader,
    build_tar_header_bytes,
    compute_file_sha256,
    extract_manifest_summary,
    verify_manifest_signature,
)
from scripts.data_access.constants import (
    DEFAULT_SYNOLOGY_FTP_BACKUP_ROOT,
    FTP_DATA_TIMEOUT_SECS,
    FTP_DEFAULT_PORT,
    FTP_PROBE_TIMEOUT_SECS,
    FTP_TRANSFER_CHUNK_BYTES,
    O2SWITCH_REMOTE_PYTHON,
    REMOTE_DBSRT_DATA_REL,
    REMOTE_O2SWITCH_BACKUPS_REL,
    REMOTE_O2SWITCH_SCRIPTS_REL,
    REMOTE_VAULT_AGENT_TIMEOUT_SECS,
    RSYNC_BACKUP_BWLIMIT_KBPS,
    RSYNC_BACKUP_TIMEOUT_SECS,
    SSH_ALIAS_O2SWITCH,
    SSH_CONNECT_TIMEOUT_SECS,
    TAR_BLOCK_BYTES,
    TAR_EOF_BLOCKS_COUNT,
)
from scripts.data_access.topology import (
    WorkspaceContext,
    inspect_network_topology,
    lookup_pass_secret,
    resolve_runtime_endpoints,
    run_bounded_cmd,
)


def compute_expected_tar_bytes(file_records: list[dict[str, Any]]) -> int:
    """Computes exact deterministic PAX tar byte length solely from manifest records."""
    total = TAR_BLOCK_BYTES * TAR_EOF_BLOCKS_COUNT
    for rec in sorted(file_records, key=lambda r: str(r["rel_path"])):
        size_b = int(rec["size_bytes"])
        hdr = build_tar_header_bytes(str(rec["rel_path"]), size_b, int(rec["mtime_epoch"]))
        pad = (TAR_BLOCK_BYTES - (size_b % TAR_BLOCK_BYTES)) % TAR_BLOCK_BYTES
        total += len(hdr) + size_b + pad
    return total


class BackupTarget(Protocol):
    """Uniform protocol implemented by all storage tier adapters."""

    name: str

    def is_reachable(self) -> bool:
        """Returns True if the target endpoint is currently reachable."""
        ...

    def push_snapshot(self, data_dir: Path, manifest: dict[str, Any], dry_run: bool = False) -> dict[str, Any]:
        """Uploads missing payloads and atomically commits the signed manifest."""
        ...

    def list_snapshots(self) -> dict[str, Any]:
        """Lists committed snapshot summaries newest-first."""
        ...

    def fetch_manifest(self, snapshot_id: str = "latest") -> dict[str, Any] | None:
        """Downloads and cryptographically verifies a full snapshot manifest."""
        ...

    def verify_snapshot(self, snapshot_id: str = "latest", check_hashes: bool = False) -> dict[str, Any]:
        """Verifies remote manifest signature and physical blob/file integrity."""
        ...

    def restore_files(
        self,
        data_dir: Path,
        manifest: dict[str, Any],
        rel_paths: set[str],
    ) -> dict[str, Any]:
        """Restores requested relative paths into data_dir after SHA-256 verification."""
        ...


def _finalize_staged_restore(
    staging_root: Path,
    data_dir: Path,
    records_by_rel: dict[str, dict[str, Any]],
) -> tuple[int, int]:
    """Verifies SHA-256 of extracted staging files and moves them atomically into data_dir."""
    restored_files = 0
    restored_bytes = 0
    for rel_path, rec in sorted(records_by_rel.items()):
        staged_file = staging_root / rel_path
        if not staged_file.is_file():
            raise FileNotFoundError(f"Restored file missing in staging area: {rel_path}")
        actual_sha = compute_file_sha256(staged_file)
        if actual_sha != str(rec["sha256"]):
            raise ValueError(f"Restore SHA-256 mismatch for {rel_path}: expected {rec['sha256']}, got {actual_sha}")
        mtime_sec = int(rec["mtime_epoch"])
        os.utime(staged_file, (mtime_sec, mtime_sec))
        dest = data_dir / rel_path
        dest.parent.mkdir(parents=True, exist_ok=True)
        os.replace(staged_file, dest)
        restored_files += 1
        restored_bytes += int(rec["size_bytes"])
    return restored_files, restored_bytes


class LocalPosixTarget:
    """POSIX hardlink vault target for local backup mounts and hermetic unit tests."""

    def __init__(self, vault_dir: Path, name: str = "local_vault", seed_data_dir: Path | None = None) -> None:
        self.name = name
        self.vault_dir = vault_dir.resolve()
        self.seed_data_dir = seed_data_dir.resolve() if seed_data_dir else None

    def is_reachable(self) -> bool:
        """Local filesystem path is reachable if parent exists or can be created."""
        return True

    def push_snapshot(self, data_dir: Path, manifest: dict[str, Any], dry_run: bool = False) -> dict[str, Any]:
        """Stages hardlinks, copies missing delta files, and commits atomically."""
        snap_id = str(manifest["snapshot_id"])
        if dry_run:
            return {
                "target": self.name,
                "status": "dry_run",
                "vault_dir": str(self.vault_dir),
                "snapshot_id": snap_id,
            }
        self.vault_dir.mkdir(parents=True, exist_ok=True)
        with tempfile.NamedTemporaryFile("w", suffix=".json", encoding="utf-8", delete=False) as tmp:
            json.dump(manifest, tmp, ensure_ascii=False)
            mpath = Path(tmp.name)
        try:
            staged = posix_vault_agent.stage_snapshot(self.vault_dir, mpath, self.seed_data_dir)
            if staged["status"] == "already_committed":
                return {"target": self.name, **staged, "uploaded_bytes": 0}
            staging_data = Path(str(staged["staging_data_dir"]))
            for rel in staged["missing_files"]:
                src = data_dir / str(rel)
                dst = staging_data / str(rel)
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(src, dst)
            committed = posix_vault_agent.commit_snapshot(self.vault_dir, mpath)
            return {
                "target": self.name,
                **committed,
                "linked_from_prev": staged["linked_from_prev"],
                "linked_from_seed": staged["linked_from_seed"],
                "uploaded_files": staged["missing_count"],
                "uploaded_bytes": staged["missing_bytes"],
            }
        finally:
            mpath.unlink(missing_ok=True)

    def list_snapshots(self) -> dict[str, Any]:
        """Lists committed snapshots from the local vault."""
        return {"target": self.name, **posix_vault_agent.list_snapshots(self.vault_dir)}

    def fetch_manifest(self, snapshot_id: str = "latest") -> dict[str, Any] | None:
        """Loads and verifies a snapshot manifest from the local vault."""
        resolved = snapshot_id
        if resolved in ("", "latest"):
            lfile = self.vault_dir / "latest.json"
            if not lfile.is_file():
                return None
            resolved = str(json.loads(lfile.read_text(encoding="utf-8")).get("snapshot_id", ""))
        mpath = self.vault_dir / "manifests" / f"{resolved}.json"
        if not mpath.is_file():
            return None
        manifest = json.loads(mpath.read_text(encoding="utf-8"))
        if not verify_manifest_signature(manifest):
            raise ValueError(f"Corrupt manifest signature on {self.name}: {resolved}")
        return manifest

    def verify_snapshot(self, snapshot_id: str = "latest", check_hashes: bool = False) -> dict[str, Any]:
        """Runs POSIX vault integrity check."""
        return {"target": self.name, **posix_vault_agent.verify_snapshot(self.vault_dir, snapshot_id, check_hashes)}

    def restore_files(
        self,
        data_dir: Path,
        manifest: dict[str, Any],
        rel_paths: set[str],
    ) -> dict[str, Any]:
        """Copies requested files into a staging directory, verifies SHA-256, and moves into data_dir."""
        snap_id = str(manifest["snapshot_id"])
        snap_data = self.vault_dir / "snapshots" / snap_id / "data"
        records_by_rel = {str(r["rel_path"]): r for r in manifest.get("files", []) if str(r["rel_path"]) in rel_paths}
        staging_root = data_dir / f".restore_tmp_{os.getpid()}"
        if staging_root.exists():
            shutil.rmtree(staging_root)
        try:
            for rel_path in sorted(records_by_rel):
                src = snap_data / rel_path
                dst = staging_root / rel_path
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(src, dst)
            count, total_b = _finalize_staged_restore(staging_root, data_dir, records_by_rel)
            return {"target": self.name, "snapshot_id": snap_id, "restored_files": count, "restored_bytes": total_b}
        finally:
            shutil.rmtree(staging_root, ignore_errors=True)


class O2SwitchRsyncTarget:
    """Cloud o2switch off-site backup target using hardlink pre-seeding + delta rsync."""

    def __init__(self, repo_root: Path, host: str = SSH_ALIAS_O2SWITCH) -> None:
        self.name = "o2switch"
        self.repo_root = repo_root
        self.host = host
        self.remote_vault = REMOTE_O2SWITCH_BACKUPS_REL
        self.remote_seed = REMOTE_DBSRT_DATA_REL
        self.remote_scripts = REMOTE_O2SWITCH_SCRIPTS_REL

    def is_reachable(self) -> bool:
        """Checks batch-mode SSH connectivity to o2switch."""
        rc, _, _ = run_bounded_cmd(
            ["ssh", "-o", "BatchMode=yes", "-o", f"ConnectTimeout={SSH_CONNECT_TIMEOUT_SECS}", self.host, "true"],
            float(SSH_CONNECT_TIMEOUT_SECS + 2),
        )
        return rc == 0

    def _run_remote_agent(self, args: list[str]) -> dict[str, Any]:
        """Executes posix_vault_agent.py on o2switch and parses its JSON stdout."""
        remote_cmd = " ".join([O2SWITCH_REMOTE_PYTHON, f"{self.remote_scripts}/posix_vault_agent.py", *args])
        rc, out, err = run_bounded_cmd(
            ["ssh", "-o", "BatchMode=yes", self.host, remote_cmd],
            REMOTE_VAULT_AGENT_TIMEOUT_SECS,
        )
        if not out:
            raise RuntimeError(f"o2switch vault agent failed (rc={rc}): {err}")
        parsed = json.loads(out.splitlines()[-1])
        if rc != 0 and parsed.get("status") == "error":
            raise RuntimeError(f"o2switch vault agent error: {parsed.get('error')}")
        return parsed

    def _ship_agent_and_manifest(self, manifest: dict[str, Any]) -> str:
        """Ships posix_vault_agent.py and temporary manifest JSON to o2switch."""
        snap_id = str(manifest["snapshot_id"])
        remote_mpath = f"{self.remote_vault}/.tmp_manifest_{snap_id}.json"
        run_bounded_cmd(
            ["ssh", "-o", "BatchMode=yes", self.host, f"mkdir -p {self.remote_scripts} {self.remote_vault}"],
            float(SSH_CONNECT_TIMEOUT_SECS + 5),
        )
        agent_src = self.repo_root / "scripts" / "data_access" / "posix_vault_agent.py"
        with tempfile.NamedTemporaryFile("w", suffix=".json", encoding="utf-8", delete=False) as tmp:
            json.dump(manifest, tmp, separators=(",", ":"), ensure_ascii=False)
            local_mpath = Path(tmp.name)
        try:
            subprocess.run(
                ["rsync", "-az", str(agent_src), f"{self.host}:{self.remote_scripts}/posix_vault_agent.py"],
                check=True,
                timeout=60,
            )
            subprocess.run(
                ["rsync", "-az", str(local_mpath), f"{self.host}:{remote_mpath}"],
                check=True,
                timeout=120,
            )
        finally:
            local_mpath.unlink(missing_ok=True)
        return remote_mpath

    def push_snapshot(self, data_dir: Path, manifest: dict[str, Any], dry_run: bool = False) -> dict[str, Any]:
        """Stages remote hardlinks against ~/dbsrt/data + previous snapshot, rsyncs delta, commits."""
        snap_id = str(manifest["snapshot_id"])
        if dry_run:
            return {
                "target": self.name,
                "status": "dry_run",
                "remote": f"{self.host}:{self.remote_vault}",
                "snapshot_id": snap_id,
            }
        remote_mpath = self._ship_agent_and_manifest(manifest)
        try:
            staged = self._run_remote_agent(
                [
                    "stage",
                    "--vault-dir",
                    self.remote_vault,
                    "--manifest",
                    remote_mpath,
                    "--seed-data-dir",
                    self.remote_seed,
                ]
            )
            if staged.get("status") == "already_committed":
                return {"target": self.name, **staged, "uploaded_bytes": 0}
            missing: list[str] = list(staged.get("missing_files", []))
            print(
                f"   [o2switch] Hardlinked {staged.get('linked_from_prev', 0)} prev + "
                f"{staged.get('linked_from_seed', 0)} seed files; "
                f"uploading {len(missing)} delta files ({staged.get('missing_bytes', 0)} bytes)...",
                file=sys.stderr,
                flush=True,
            )
            if missing:
                with tempfile.NamedTemporaryFile("w", suffix=".txt", encoding="utf-8", delete=False) as flist:
                    flist.write("\n".join(missing) + "\n")
                    flist_path = Path(flist.name)
                try:
                    remote_dest = f"{self.host}:{self.remote_vault}/snapshots/.staging_{snap_id}/data/"
                    subprocess.run(
                        [
                            "rsync",
                            "-az",
                            f"--bwlimit={RSYNC_BACKUP_BWLIMIT_KBPS}",
                            f"--files-from={flist_path}",
                            f"{data_dir}/",
                            remote_dest,
                        ],
                        check=True,
                        timeout=RSYNC_BACKUP_TIMEOUT_SECS,
                    )
                finally:
                    flist_path.unlink(missing_ok=True)
            committed = self._run_remote_agent(["commit", "--vault-dir", self.remote_vault, "--manifest", remote_mpath])
            return {
                "target": self.name,
                **committed,
                "linked_from_prev": staged.get("linked_from_prev", 0),
                "linked_from_seed": staged.get("linked_from_seed", 0),
                "uploaded_files": len(missing),
                "uploaded_bytes": staged.get("missing_bytes", 0),
            }
        finally:
            run_bounded_cmd(
                ["ssh", "-o", "BatchMode=yes", self.host, f"rm -f {remote_mpath}"],
                float(SSH_CONNECT_TIMEOUT_SECS + 5),
            )

    def list_snapshots(self) -> dict[str, Any]:
        """Lists committed snapshots on o2switch."""
        agent_src = self.repo_root / "scripts" / "data_access" / "posix_vault_agent.py"
        run_bounded_cmd(
            ["ssh", "-o", "BatchMode=yes", self.host, f"mkdir -p {self.remote_scripts} {self.remote_vault}"],
            float(SSH_CONNECT_TIMEOUT_SECS + 5),
        )
        subprocess.run(
            ["rsync", "-az", str(agent_src), f"{self.host}:{self.remote_scripts}/posix_vault_agent.py"],
            check=True,
            timeout=30,
        )
        return {"target": self.name, **self._run_remote_agent(["list", "--vault-dir", self.remote_vault])}

    def fetch_manifest(self, snapshot_id: str = "latest") -> dict[str, Any] | None:
        """Downloads and cryptographically verifies a snapshot manifest from o2switch."""
        resolved = snapshot_id
        if resolved in ("", "latest"):
            rc, out, _ = run_bounded_cmd(
                ["ssh", "-o", "BatchMode=yes", self.host, f"cat {self.remote_vault}/latest.json 2>/dev/null"],
                15.0,
            )
            if rc != 0 or not out:
                return None
            resolved = str(json.loads(out).get("snapshot_id", ""))
        if not resolved:
            return None
        with tempfile.NamedTemporaryFile("wb", suffix=".json", delete=False) as tmp:
            tmp_path = Path(tmp.name)
        try:
            proc = subprocess.run(
                ["rsync", "-az", f"{self.host}:{self.remote_vault}/manifests/{resolved}.json", str(tmp_path)],
                capture_output=True,
                timeout=60,
                check=False,
            )
            if proc.returncode != 0 or not tmp_path.is_file() or tmp_path.stat().st_size == 0:
                return None
            manifest = json.loads(tmp_path.read_text(encoding="utf-8"))
            if not verify_manifest_signature(manifest):
                raise ValueError(f"Corrupt manifest signature on o2switch: {resolved}")
            return manifest
        finally:
            tmp_path.unlink(missing_ok=True)

    def verify_snapshot(self, snapshot_id: str = "latest", check_hashes: bool = False) -> dict[str, Any]:
        """Runs remote vault integrity check on o2switch."""
        cmd = ["verify", "--vault-dir", self.remote_vault, "--snapshot-id", snapshot_id]
        if check_hashes:
            cmd.append("--check-hashes")
        return {"target": self.name, **self._run_remote_agent(cmd)}

    def restore_files(
        self,
        data_dir: Path,
        manifest: dict[str, Any],
        rel_paths: set[str],
    ) -> dict[str, Any]:
        """Pulls requested files via rsync into staging, verifies SHA-256, and moves into data_dir."""
        snap_id = str(manifest["snapshot_id"])
        records_by_rel = {str(r["rel_path"]): r for r in manifest.get("files", []) if str(r["rel_path"]) in rel_paths}
        staging_root = data_dir / f".restore_tmp_{os.getpid()}"
        staging_root.mkdir(parents=True, exist_ok=True)
        with tempfile.NamedTemporaryFile("w", suffix=".txt", encoding="utf-8", delete=False) as flist:
            flist.write("\n".join(sorted(records_by_rel)) + "\n")
            flist_path = Path(flist.name)
        try:
            remote_src = f"{self.host}:{self.remote_vault}/snapshots/{snap_id}/data/"
            subprocess.run(
                [
                    "rsync",
                    "-az",
                    f"--bwlimit={RSYNC_BACKUP_BWLIMIT_KBPS}",
                    f"--files-from={flist_path}",
                    remote_src,
                    f"{staging_root}/",
                ],
                check=True,
                timeout=RSYNC_BACKUP_TIMEOUT_SECS,
            )
            count, total_b = _finalize_staged_restore(staging_root, data_dir, records_by_rel)
            return {"target": self.name, "snapshot_id": snap_id, "restored_files": count, "restored_bytes": total_b}
        finally:
            flist_path.unlink(missing_ok=True)
            shutil.rmtree(staging_root, ignore_errors=True)


@dataclass(frozen=True)
class FtpConnectionConfig:
    """Connection parameters for Synology NAS userspace FTP backup vault."""

    host: str
    port: int
    user: str
    password: str
    base_dir: str = DEFAULT_SYNOLOGY_FTP_BACKUP_ROOT


class SynologyFtpTarget:
    """Userspace FTP backup target using deduplicated deterministic TAR dataset packs (zero CIFS)."""

    def __init__(self, cfg: FtpConnectionConfig) -> None:
        self.name = "synology_nas"
        self.cfg = cfg

    def _connect(self, timeout: float = FTP_DATA_TIMEOUT_SECS) -> ftplib.FTP:
        """Opens an authenticated passive-mode UTF-8 FTP session."""
        ftp = ftplib.FTP()
        ftp.encoding = "utf-8"
        ftp.connect(self.cfg.host, self.cfg.port, timeout=timeout)
        ftp.login(self.cfg.user, self.cfg.password)
        ftp.set_pasv(True)
        return ftp

    def is_reachable(self) -> bool:
        """Probes FTP authentication without mounting kernel CIFS."""
        if not self.cfg.user or not self.cfg.password or "<" in self.cfg.host:
            return False
        try:
            with self._connect(timeout=FTP_PROBE_TIMEOUT_SECS) as ftp:
                ftp.pwd()
            return True
        except ftplib.all_errors:
            return False

    def _ensure_dirs(self, ftp: ftplib.FTP) -> None:
        """Creates base_dir, packs/, and manifests/ idempotently."""
        cur = ""
        for part in [p for p in self.cfg.base_dir.strip("/").split("/") if p]:
            cur = f"{cur}/{part}"
            try:
                ftp.mkd(cur)
            except ftplib.error_perm:
                pass
        for sub in ("packs", "manifests"):
            try:
                ftp.mkd(f"{self.cfg.base_dir}/{sub}")
            except ftplib.error_perm:
                pass

    def _list_pack_sizes(self, ftp: ftplib.FTP) -> dict[str, int]:
        """Lists remote pack filenames and exact byte sizes via MLSD (or SIZE fallback)."""
        packs_dir = f"{self.cfg.base_dir}/packs"
        sizes: dict[str, int] = {}
        try:
            for name, facts in ftp.mlsd(packs_dir):
                if facts.get("type") == "file" and "size" in facts:
                    sizes[name] = int(facts["size"])
            return sizes
        except ftplib.all_errors:
            pass
        try:
            for item in ftp.nlst(packs_dir):
                bname = item.rsplit("/", 1)[-1]
                sz = ftp.size(f"{packs_dir}/{bname}")
                if sz is not None:
                    sizes[bname] = int(sz)
        except ftplib.all_errors:
            pass
        return sizes

    def _atomic_upload_bytes(self, ftp: ftplib.FTP, remote_path: str, data: bytes) -> None:
        """Uploads small JSON metadata via a .partial sibling and atomic RNFR/RNTO."""
        tmp_path = f"{remote_path}.partial.{os.getpid()}"
        with io.BytesIO(data) as bio:
            ftp.storbinary(f"STOR {tmp_path}", bio, blocksize=FTP_TRANSFER_CHUNK_BYTES)
        remote_sz = ftp.size(tmp_path)
        if remote_sz is not None and int(remote_sz) != len(data):
            raise OSError(f"FTP size mismatch on {tmp_path}: expected {len(data)}, got {remote_sz}")
        try:
            ftp.delete(remote_path)
        except ftplib.error_perm:
            pass
        ftp.rename(tmp_path, remote_path)

    def _upload_dataset_pack(
        self,
        ftp: ftplib.FTP,
        data_dir: Path,
        pack_name: str,
        ds_files: list[dict[str, Any]],
    ) -> int:
        """Streams a deterministic tar pack directly from source files into FTP STOR (0 temp disk)."""
        remote_final = f"{self.cfg.base_dir}/packs/{pack_name}"
        remote_tmp = f"{remote_final}.partial.{os.getpid()}"
        with DeterministicTarStreamReader(data_dir, ds_files) as reader:
            expected_b = reader.expected_total_bytes
            ftp.storbinary(f"STOR {remote_tmp}", reader, blocksize=FTP_TRANSFER_CHUNK_BYTES)
        actual_b = ftp.size(remote_tmp)
        if actual_b is None or int(actual_b) != expected_b:
            raise OSError(f"FTP pack size mismatch for {pack_name}: expected {expected_b}, got {actual_b}")
        try:
            ftp.delete(remote_final)
        except ftplib.error_perm:
            pass
        ftp.rename(remote_tmp, remote_final)
        return expected_b

    def push_snapshot(self, data_dir: Path, manifest: dict[str, Any], dry_run: bool = False) -> dict[str, Any]:
        """Uploads missing dataset packs, verifies exact byte lengths, and commits manifest last."""
        snap_id = str(manifest["snapshot_id"])
        if dry_run:
            return {
                "target": self.name,
                "status": "dry_run",
                "remote": f"ftp://{self.cfg.host}:{self.cfg.port}{self.cfg.base_dir}",
                "snapshot_id": snap_id,
            }
        files_by_ds: dict[str, list[dict[str, Any]]] = {}
        for rec in manifest.get("files", []):
            files_by_ds.setdefault(str(rec["dataset"]), []).append(rec)

        uploaded_packs = 0
        reused_packs = 0
        uploaded_bytes = 0

        with self._connect() as ftp:
            ftp.voidcmd("TYPE I")
            self._ensure_dirs(ftp)
            existing_sizes = self._list_pack_sizes(ftp)

            for ds_id, ds_info in manifest.get("datasets", {}).items():
                if not ds_info.get("present"):
                    continue
                pack_name = str(ds_info["pack_filename"])
                ds_files = files_by_ds.get(ds_id, [])
                exp_tar_b = compute_expected_tar_bytes(ds_files)
                if existing_sizes.get(pack_name) == exp_tar_b:
                    reused_packs += 1
                    print(
                        f"   [synology_nas] Reusing existing pack {pack_name} ({exp_tar_b} bytes)",
                        file=sys.stderr,
                        flush=True,
                    )
                    continue
                print(
                    f"   [synology_nas] Streaming pack {pack_name} ({exp_tar_b} bytes)...", file=sys.stderr, flush=True
                )
                sent_b = self._upload_dataset_pack(ftp, data_dir, pack_name, ds_files)
                uploaded_packs += 1
                uploaded_bytes += sent_b

            m_bytes = json.dumps(manifest, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
            summary = extract_manifest_summary(manifest)
            s_bytes = json.dumps(summary, indent=2, ensure_ascii=False).encode("utf-8")
            latest_bytes = json.dumps(
                {
                    "snapshot_id": snap_id,
                    "created_at_iso": manifest["created_at_iso"],
                    "git_commit": manifest.get("git_commit", ""),
                    "total_files": manifest.get("total_files", 0),
                    "total_bytes": manifest.get("total_bytes", 0),
                    "manifest_body_sha256": manifest["manifest_body_sha256"],
                },
                indent=2,
                ensure_ascii=False,
            ).encode("utf-8")

            self._atomic_upload_bytes(ftp, f"{self.cfg.base_dir}/manifests/{snap_id}.json", m_bytes)
            self._atomic_upload_bytes(ftp, f"{self.cfg.base_dir}/manifests/{snap_id}.summary.json", s_bytes)
            self._atomic_upload_bytes(ftp, f"{self.cfg.base_dir}/latest.json", latest_bytes)

        return {
            "target": self.name,
            "status": "committed",
            "snapshot_id": snap_id,
            "uploaded_packs": uploaded_packs,
            "reused_packs": reused_packs,
            "uploaded_bytes": uploaded_bytes,
        }

    def _download_bytes(self, ftp: ftplib.FTP, remote_path: str) -> bytes | None:
        """Downloads a small JSON file into memory, returning None if absent."""
        buf = io.BytesIO()
        try:
            ftp.retrbinary(f"RETR {remote_path}", buf.write, blocksize=FTP_TRANSFER_CHUNK_BYTES)
            return buf.getvalue()
        except ftplib.error_perm:
            return None

    def list_snapshots(self) -> dict[str, Any]:
        """Lists committed snapshot summaries on Synology FTP."""
        with self._connect() as ftp:
            latest_raw = self._download_bytes(ftp, f"{self.cfg.base_dir}/latest.json")
            latest_id = str(json.loads(latest_raw.decode("utf-8")).get("snapshot_id", "")) if latest_raw else ""
            summaries: list[dict[str, Any]] = []
            try:
                names = [n.rsplit("/", 1)[-1] for n in ftp.nlst(f"{self.cfg.base_dir}/manifests")]
            except ftplib.all_errors:
                names = []
            for sname in sorted((n for n in names if n.endswith(".summary.json")), reverse=True):
                raw = self._download_bytes(ftp, f"{self.cfg.base_dir}/manifests/{sname}")
                if raw:
                    summaries.append(json.loads(raw.decode("utf-8")))
        return {
            "target": self.name,
            "vault_dir": f"ftp://{self.cfg.host}:{self.cfg.port}{self.cfg.base_dir}",
            "latest_snapshot_id": latest_id,
            "snapshot_count": len(summaries),
            "snapshots": summaries,
        }

    def fetch_manifest(self, snapshot_id: str = "latest") -> dict[str, Any] | None:
        """Downloads and cryptographically verifies a full manifest from Synology FTP."""
        with self._connect() as ftp:
            resolved = snapshot_id
            if resolved in ("", "latest"):
                lraw = self._download_bytes(ftp, f"{self.cfg.base_dir}/latest.json")
                if not lraw:
                    return None
                resolved = str(json.loads(lraw.decode("utf-8")).get("snapshot_id", ""))
            if not resolved:
                return None
            mraw = self._download_bytes(ftp, f"{self.cfg.base_dir}/manifests/{resolved}.json")
            if not mraw:
                return None
            manifest = json.loads(mraw.decode("utf-8"))
            if not verify_manifest_signature(manifest):
                raise ValueError(f"Corrupt manifest signature on Synology FTP: {resolved}")
            return manifest

    def verify_snapshot(self, snapshot_id: str = "latest", check_hashes: bool = False) -> dict[str, Any]:
        """Verifies manifest signature and exact deterministic TAR pack sizes on Synology FTP."""
        manifest = self.fetch_manifest(snapshot_id)
        if manifest is None:
            return {
                "target": self.name,
                "status": "missing",
                "snapshot_id": snapshot_id,
                "errors": ["Manifest not found"],
            }
        snap_id = str(manifest["snapshot_id"])
        files_by_ds: dict[str, list[dict[str, Any]]] = {}
        for rec in manifest.get("files", []):
            files_by_ds.setdefault(str(rec["dataset"]), []).append(rec)

        errors: list[str] = []
        checked_packs = 0
        with self._connect() as ftp:
            ftp.voidcmd("TYPE I")
            pack_sizes = self._list_pack_sizes(ftp)
            for ds_id, ds_info in manifest.get("datasets", {}).items():
                if not ds_info.get("present"):
                    continue
                pack_name = str(ds_info["pack_filename"])
                exp_b = compute_expected_tar_bytes(files_by_ds.get(ds_id, []))
                actual_b = pack_sizes.get(pack_name)
                if actual_b is None:
                    errors.append(f"missing_pack:{pack_name}")
                elif actual_b != exp_b:
                    errors.append(f"pack_size_mismatch:{pack_name}:{actual_b}!={exp_b}")
                else:
                    checked_packs += 1
                    if check_hashes:
                        self._verify_pack_member_hashes(ftp, pack_name, files_by_ds.get(ds_id, []), errors)

        return {
            "target": self.name,
            "status": "ok" if not errors else "corrupt",
            "snapshot_id": snap_id,
            "manifest_body_sha256": manifest["manifest_body_sha256"],
            "checked_packs": checked_packs,
            "checked_files": manifest.get("total_files", 0) if not errors else 0,
            "hashes_verified": check_hashes,
            "errors": errors,
        }

    def _verify_pack_member_hashes(
        self,
        ftp: ftplib.FTP,
        pack_name: str,
        ds_files: list[dict[str, Any]],
        errors: list[str],
    ) -> None:
        """Streams a remote pack over RETR and checks SHA-256 of every contained file."""
        import hashlib

        expected = {str(r["rel_path"]): str(r["sha256"]) for r in ds_files}
        seen: set[str] = set()
        remote_pack = f"{self.cfg.base_dir}/packs/{pack_name}"
        with ftp.transfercmd(f"RETR {remote_pack}") as sock:
            with sock.makefile("rb", buffering=FTP_TRANSFER_CHUNK_BYTES) as stream:
                with tarfile.open(fileobj=stream, mode="r|") as tf:
                    for member in tf:
                        if not member.isfile():
                            continue
                        extracted = tf.extractfile(member)
                        if extracted is None:
                            continue
                        digest = hashlib.sha256()
                        while True:
                            chunk = extracted.read(FTP_TRANSFER_CHUNK_BYTES)
                            if not chunk:
                                break
                            digest.update(chunk)
                        seen.add(member.name)
                        if expected.get(member.name) != digest.hexdigest():
                            errors.append(f"sha256_mismatch:{member.name}")
                while stream.read(FTP_TRANSFER_CHUNK_BYTES):
                    pass
        ftp.voidresp()
        for missing_rel in sorted(set(expected) - seen):
            errors.append(f"missing_in_pack:{missing_rel}")

    def _extract_pack_members(
        self,
        ftp: ftplib.FTP,
        pack_name: str,
        wanted: set[str],
        staging_root: Path,
    ) -> None:
        """Streams remote pack and writes only wanted relative paths into staging_root."""
        resolved_root = staging_root.resolve()
        remote_pack = f"{self.cfg.base_dir}/packs/{pack_name}"
        with ftp.transfercmd(f"RETR {remote_pack}") as sock:
            with sock.makefile("rb", buffering=FTP_TRANSFER_CHUNK_BYTES) as stream:
                with tarfile.open(fileobj=stream, mode="r|") as tf:
                    for member in tf:
                        if not member.isfile() or member.name not in wanted:
                            continue
                        dst = (resolved_root / member.name).resolve()
                        dst.relative_to(resolved_root)
                        src = tf.extractfile(member)
                        if src is None:
                            continue
                        dst.parent.mkdir(parents=True, exist_ok=True)
                        with dst.open("wb") as out_fh:
                            shutil.copyfileobj(src, out_fh, length=FTP_TRANSFER_CHUNK_BYTES)
                while stream.read(FTP_TRANSFER_CHUNK_BYTES):
                    pass
        ftp.voidresp()

    def restore_files(
        self,
        data_dir: Path,
        manifest: dict[str, Any],
        rel_paths: set[str],
    ) -> dict[str, Any]:
        """Streams required dataset packs from FTP, verifies SHA-256, and moves into data_dir."""
        snap_id = str(manifest["snapshot_id"])
        records_by_rel = {str(r["rel_path"]): r for r in manifest.get("files", []) if str(r["rel_path"]) in rel_paths}
        wanted_by_ds: dict[str, set[str]] = {}
        for rel, rec in records_by_rel.items():
            wanted_by_ds.setdefault(str(rec["dataset"]), set()).add(rel)

        staging_root = data_dir / f".restore_tmp_{os.getpid()}"
        if staging_root.exists():
            shutil.rmtree(staging_root)
        staging_root.mkdir(parents=True, exist_ok=True)
        try:
            with self._connect() as ftp:
                ftp.voidcmd("TYPE I")
                for ds_id, wanted_rels in sorted(wanted_by_ds.items()):
                    pack_name = str(manifest["datasets"][ds_id]["pack_filename"])
                    self._extract_pack_members(ftp, pack_name, wanted_rels, staging_root)
            count, total_b = _finalize_staged_restore(staging_root, data_dir, records_by_rel)
            return {"target": self.name, "snapshot_id": snap_id, "restored_files": count, "restored_bytes": total_b}
        finally:
            shutil.rmtree(staging_root, ignore_errors=True)


def build_configured_targets(
    ws: WorkspaceContext,
    requested_targets: list[str] | None = None,
) -> list[BackupTarget]:
    """Resolves active backup targets (1A default: o2switch + synology_nas when reachable)."""
    ep = resolve_runtime_endpoints(ws)
    net = inspect_network_topology()
    nas_host = ep.nas_lan_host if net.get("profile") == "home_lan" else ep.nas_wan_host
    nas_pw = lookup_pass_secret(ws.env_vars, "NAS_PASS", ep.nas_pass_entry)
    nas_root = ws.env_vars.get("SYNOLOGY_FTP_BACKUP_ROOT", DEFAULT_SYNOLOGY_FTP_BACKUP_ROOT)

    specs = requested_targets or ["o2switch", "synology_nas"]
    resolved: list[BackupTarget] = []
    for raw in specs:
        token = raw.strip()
        if token.startswith("local:"):
            vault_path = Path(token.split(":", 1)[1]).expanduser()
            resolved.append(LocalPosixTarget(vault_path, name=f"local:{vault_path.name}"))
        elif token in ("o2switch", "cloud"):
            resolved.append(O2SwitchRsyncTarget(ws.common_repo))
        elif token in ("synology_nas", "synology", "nas"):
            resolved.append(
                SynologyFtpTarget(
                    FtpConnectionConfig(
                        host=nas_host,
                        port=FTP_DEFAULT_PORT,
                        user=ep.nas_user,
                        password=nas_pw,
                        base_dir=nas_root,
                    )
                )
            )
        else:
            raise ValueError(f"Unknown backup target '{token}'. Expected o2switch, synology_nas, or local:<dir>.")
    return resolved
