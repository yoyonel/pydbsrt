"""Hermetic unit and loopback-FTP integration tests for multi-tier SHA-256 dataset backups."""

from __future__ import annotations

import io
import os
import socket
import socketserver
import struct
import tarfile
import threading
from pathlib import Path

import pytest

from scripts.data_access.backup_cli import (
    run_backup_cmd,
    run_evict_cmd,
    run_restore_cmd,
    run_verify_cmd,
)
from scripts.data_access.backup_manifest import (
    DeterministicTarStreamReader,
    build_backup_manifest,
    inspect_local_backup_health,
    load_evicted_manifest,
    verify_manifest_signature,
)
from scripts.data_access.backup_targets import (
    FtpConnectionConfig,
    LocalPosixTarget,
    SynologyFtpTarget,
    compute_expected_tar_bytes,
)
from scripts.data_access.constants import (
    DBAHASH_HEADER_PREFIX_FORMAT,
    DBAHASH_MAGIC_PREFIX,
    DBPHASH_HEADER_PREFIX_FORMAT,
    DBPHASH_MAGIC_PREFIX,
    EXIT_OK,
    EXIT_STALE_OR_DRIFT,
)


def _populate_sample_data_dir(data_dir: Path) -> None:
    """Creates synthetic binary catalogs, UTF-8 snippets, couplings, and transient files."""
    data_dir.mkdir(parents=True, exist_ok=True)
    dbphash_hdr = struct.pack(DBPHASH_HEADER_PREFIX_FORMAT, DBPHASH_MAGIC_PREFIX + b"\x00\x00\x00", 2, 0, 42)
    (data_dir / "catalog.dbphash").write_bytes(dbphash_hdr + b"P" * 64)

    dbahash_hdr = struct.pack(DBAHASH_HEADER_PREFIX_FORMAT, DBAHASH_MAGIC_PREFIX, 1, 0, 8, 17)
    (data_dir / "catalog.dbahash").write_bytes(dbahash_hdr + b"A" * 128)

    srt_dir = data_dir / "SRT"
    srt_dir.mkdir()
    (srt_dir / "couplings.json").write_text('{"a": 1, "b": 2, "c": 3}', encoding="utf-8")
    (srt_dir / "sample.fr.srt").write_text("1\n00:00:01,000 --> 00:00:02,000\nBonjour\n", encoding="utf-8")

    snip_dir = data_dir / "snippets"
    snip_dir.mkdir()
    (snip_dir / "Comédie_éàô_mid15s.mp4").write_bytes(b"\x00\x00\x00\x18ftypmp42" + b"V" * 1500)
    (snip_dir / "ignored.mp4.partial").write_bytes(b"transient")

    bench_dir = data_dir / "benchmarks" / "sub"
    bench_dir.mkdir(parents=True)
    (bench_dir / "score.json").write_text('{"acc": 1.0}', encoding="utf-8")


def test_manifest_signature_headers_and_tar_stream(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """Verifies manifest signature, .dbphash/.dbahash counts, transient filtering, and PAX tar stream."""
    monkeypatch.setenv("DBSRT_BACKUP_HASH_CACHE", str(tmp_path / "hash_cache.json"))
    data_dir = tmp_path / "data"
    _populate_sample_data_dir(data_dir)

    manifest = build_backup_manifest(data_dir, git_commit="abc1234")
    assert verify_manifest_signature(manifest) is True
    assert manifest["datasets"]["catalog_dbphash"]["domain_meta"]["media_count"] == 42
    assert manifest["datasets"]["catalog_dbahash"]["domain_meta"]["media_count"] == 17
    assert manifest["datasets"]["srt_store"]["domain_meta"]["couplings_count"] == 3

    rel_paths = {r["rel_path"] for r in manifest["files"]}
    assert "snippets/Comédie_éàô_mid15s.mp4" in rel_paths
    assert "snippets/ignored.mp4.partial" not in rel_paths

    snip_files = [r for r in manifest["files"] if r["dataset"] == "snippets"]
    with DeterministicTarStreamReader(data_dir, snip_files) as reader:
        chunks: list[bytes] = []
        while True:
            piece = reader.read(137)
            if not piece:
                break
            chunks.append(piece)
        raw_tar = b"".join(chunks)
        assert len(raw_tar) == reader.expected_total_bytes == compute_expected_tar_bytes(snip_files)

    with tarfile.open(fileobj=io.BytesIO(raw_tar), mode="r|") as tf:
        members = [(m.name, tf.extractfile(m).read()) for m in tf if m.isfile()]  # type: ignore[union-attr]
    assert len(members) == 1
    assert members[0][0] == "snippets/Comédie_éàô_mid15s.mp4"
    assert members[0][1] == (data_dir / "snippets/Comédie_éàô_mid15s.mp4").read_bytes()

    tampered = dict(manifest)
    tampered["total_bytes"] = int(manifest["total_bytes"]) + 1
    assert verify_manifest_signature(tampered) is False


def test_posix_vault_seed_hardlinks_and_point_in_time_restore(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """Proves hardlink pre-seeding (0 copy from seed dir) and historical snapshot rollback."""
    monkeypatch.setenv("DBSRT_BACKUP_HASH_CACHE", str(tmp_path / "hash_cache.json"))
    monkeypatch.setenv("DBSRT_LATEST_BACKUP_CACHE", str(tmp_path / "latest_bak.json"))
    data_dir = tmp_path / "data"
    _populate_sample_data_dir(data_dir)

    seed_dir = tmp_path / "o2switch_live_data"
    seed_dir.mkdir()
    (seed_dir / "catalog.dbahash").write_bytes((data_dir / "catalog.dbahash").read_bytes())

    vault = LocalPosixTarget(tmp_path / "vault", name="local:v1", seed_data_dir=seed_dir)
    rc1, res1 = run_backup_cmd(data_dir, [vault], None, (False, False))
    assert rc1 == EXIT_OK
    t1 = res1["target_results"][0]
    assert t1["linked_from_seed"] == 1
    snap1_id = res1["snapshot"]["snapshot_id"]

    seed_ino = (seed_dir / "catalog.dbahash").stat().st_ino
    snap1_ino = (tmp_path / "vault" / "snapshots" / snap1_id / "data" / "catalog.dbahash").stat().st_ino
    assert seed_ino == snap1_ino

    score_file = data_dir / "benchmarks" / "sub" / "score.json"
    score_file.write_text('{"acc": 0.5}', encoding="utf-8")
    m2 = build_backup_manifest(data_dir, git_commit="def5678")
    m2["snapshot_id"] = f"{snap1_id}_v2"
    from scripts.data_access.backup_manifest import compute_manifest_body_sha256

    m2["manifest_body_sha256"] = compute_manifest_body_sha256(m2)
    t2 = vault.push_snapshot(data_dir, m2)
    assert t2["linked_from_prev"] == len(m2["files"]) - 1
    assert t2["uploaded_files"] == 1

    rc_res, restored = run_restore_cmd(data_dir, [vault], (["benchmarks"], [], snap1_id, False))
    assert rc_res == EXIT_OK
    assert restored["restored_files"] == 1
    assert score_file.read_text(encoding="utf-8") == '{"acc": 1.0}'


class _MiniFtpHandler(socketserver.StreamRequestHandler):
    """Minimal RFC 959 / RFC 3659 passive loopback FTP server for hermetic Synology tests."""

    root_dir: Path

    def _send(self, line: str) -> None:
        self.wfile.write((line + "\r\n").encode("utf-8"))
        self.wfile.flush()

    def _resolve(self, ftp_path: str) -> Path:
        clean = ftp_path.strip().lstrip("/")
        resolved = (self.root_dir / clean).resolve()
        resolved.relative_to(self.root_dir.resolve())
        return resolved

    def handle(self) -> None:
        self._send("220 MiniFTP ready")
        cwd = "/"
        rnfr_src: Path | None = None
        pasv_srv: socket.socket | None = None

        try:
            while True:
                raw = self.rfile.readline()
                if not raw:
                    break
                line = raw.decode("utf-8").rstrip("\r\n")
                parts = line.split(" ", 1)
                cmd = parts[0].upper()
                arg = parts[1] if len(parts) > 1 else ""

                if cmd in ("USER", "PASS", "TYPE"):
                    self._send("230 OK" if cmd == "PASS" else ("331 Password" if cmd == "USER" else "200 Binary"))
                elif cmd == "PWD":
                    self._send(f'257 "{cwd}"')
                elif cmd == "MKD":
                    p = self._resolve(arg)
                    if p.exists():
                        self._send("550 Exists")
                    else:
                        p.mkdir(parents=True, exist_ok=True)
                        self._send(f'257 "{arg}"')
                elif cmd == "SIZE":
                    p = self._resolve(arg)
                    self._send(f"213 {p.stat().st_size}" if p.is_file() else "550 Missing")
                elif cmd == "DELE":
                    p = self._resolve(arg)
                    if p.is_file():
                        p.unlink()
                        self._send("250 Deleted")
                    else:
                        self._send("550 Missing")
                elif cmd == "RNFR":
                    rnfr_src = self._resolve(arg)
                    self._send("350 Ready" if rnfr_src.exists() else "550 Missing")
                elif cmd == "RNTO":
                    dst = self._resolve(arg)
                    if rnfr_src is not None and rnfr_src.exists():
                        dst.parent.mkdir(parents=True, exist_ok=True)
                        os.replace(rnfr_src, dst)
                        rnfr_src = None
                        self._send("250 Renamed")
                    else:
                        self._send("550 Failed")
                elif cmd == "PASV":
                    if pasv_srv is not None:
                        pasv_srv.close()
                    pasv_srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
                    pasv_srv.bind(("127.0.0.1", 0))
                    pasv_srv.listen(1)
                    port = pasv_srv.getsockname()[1]
                    self._send(f"227 Entering Passive Mode (127,0,0,{1},{port >> 8},{port & 0xFF}).")
                elif cmd in ("STOR", "RETR", "MLSD", "NLST"):
                    assert pasv_srv is not None
                    self._send("150 Opening data connection")
                    conn, _ = pasv_srv.accept()
                    with conn:
                        p = self._resolve(arg)
                        if cmd == "STOR":
                            p.parent.mkdir(parents=True, exist_ok=True)
                            with p.open("wb") as fh:
                                while True:
                                    buf = conn.recv(65536)
                                    if not buf:
                                        break
                                    fh.write(buf)
                        elif cmd == "RETR":
                            if not p.is_file():
                                self._send("550 Missing")
                                continue
                            conn.sendall(p.read_bytes())
                        elif cmd == "MLSD":
                            if p.is_dir():
                                lines = [
                                    f"type={'file' if c.is_file() else 'dir'};size={c.stat().st_size if c.is_file() else 0}; {c.name}\r\n"
                                    for c in sorted(p.iterdir())
                                ]
                                conn.sendall("".join(lines).encode("utf-8"))
                        elif cmd == "NLST" and p.is_dir():
                            lines = [f"{c.name}\r\n" for c in sorted(p.iterdir())]
                            conn.sendall("".join(lines).encode("utf-8"))
                    pasv_srv.close()
                    pasv_srv = None
                    self._send("226 Transfer complete")
                elif cmd == "QUIT":
                    self._send("221 Bye")
                    break
                else:
                    self._send("502 Not implemented")
        finally:
            if pasv_srv is not None:
                pasv_srv.close()


def test_synology_ftp_pack_dedup_corruption_and_evict_restore(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    """Tests SynologyFtpTarget over real passive TCP sockets + multi-replica eviction guards."""
    monkeypatch.setenv("DBSRT_BACKUP_HASH_CACHE", str(tmp_path / "hash_cache.json"))
    monkeypatch.setenv("DBSRT_LATEST_BACKUP_CACHE", str(tmp_path / "latest_bak.json"))

    data_dir = tmp_path / "data"
    _populate_sample_data_dir(data_dir)
    ftp_root = tmp_path / "ftp_server_root"
    ftp_root.mkdir()

    class _BoundHandler(_MiniFtpHandler):
        root_dir = ftp_root

    server = socketserver.ThreadingTCPServer(("127.0.0.1", 0), _BoundHandler)
    server.daemon_threads = True
    port = server.server_address[1]
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()

    try:
        nas_tgt = SynologyFtpTarget(
            FtpConnectionConfig(
                host="127.0.0.1",
                port=port,
                user="testuser",
                password="secret",
                base_dir="/home/pydbsrt-backups",
            )
        )
        posix_tgt = LocalPosixTarget(tmp_path / "o2s_mock_vault", name="o2switch")

        rc_bak, bak_out = run_backup_cmd(data_dir, [posix_tgt, nas_tgt], None, (False, False))
        assert rc_bak == EXIT_OK
        assert bak_out["replicas_committed"] == ["o2switch", "synology_nas"]
        nas_res1 = bak_out["target_results"][1]
        assert nas_res1["uploaded_packs"] == 5
        assert nas_res1["reused_packs"] == 0

        rc_ver, ver_out = run_verify_cmd([posix_tgt, nas_tgt], "latest", check_hashes=True)
        assert rc_ver == EXIT_OK
        assert ver_out["status"] == "ok"

        # 2nd backup with zero changes reuses 100% of FTP packs (0 uploaded bytes)
        m_repeat = build_backup_manifest(data_dir, git_commit="repeat1")
        nas_res2 = nas_tgt.push_snapshot(data_dir, m_repeat)
        assert nas_res2["uploaded_packs"] == 0
        assert nas_res2["reused_packs"] == 5
        assert nas_res2["uploaded_bytes"] == 0

        # Guard 1: Eviction with 1 replica when min_replicas=2 MUST block
        rc_blk1, blk1 = run_evict_cmd(data_dir, [nas_tgt], ["benchmarks"], ("latest", 2, True))
        assert rc_blk1 == EXIT_STALE_OR_DRIFT
        assert blk1["status"] == "blocked_unverified_replica"
        assert (data_dir / "benchmarks" / "sub" / "score.json").is_file()

        # Guard 2: Silent local byte mutation of equal length MUST block eviction
        score_path = data_dir / "benchmarks" / "sub" / "score.json"
        score_path.write_text('{"acc": 0.9}', encoding="utf-8")
        rc_blk2, blk2 = run_evict_cmd(data_dir, [posix_tgt, nas_tgt], ["benchmarks"], ("latest", 2, True))
        assert rc_blk2 == EXIT_STALE_OR_DRIFT
        assert "hash_drift" in blk2["diagnostics"]["o2switch"]
        score_path.write_text('{"acc": 1.0}', encoding="utf-8")

        # Guard 3: Dry-run eviction leaves files intact
        rc_dry, dry_out = run_evict_cmd(data_dir, [posix_tgt, nas_tgt], ["benchmarks"], ("latest", 2, False))
        assert rc_dry == EXIT_OK
        assert dry_out["status"] == "dry_run_ready_to_evict"
        assert score_path.is_file()

        # Verified 2-replica eviction deletes files and records tombstone
        rc_ev, ev_out = run_evict_cmd(data_dir, [posix_tgt, nas_tgt], ["benchmarks"], ("latest", 2, True))
        assert rc_ev == EXIT_OK
        assert ev_out["status"] == "evicted"
        assert not score_path.exists()
        assert "benchmarks" in load_evicted_manifest(data_dir)["evicted_datasets"]
        assert "benchmarks" in inspect_local_backup_health(data_dir)["evicted_datasets"]

        # Restore from Synology FTP alone verifies SHA-256 and clears tombstone
        rc_res, res_out = run_restore_cmd(data_dir, [nas_tgt], (["benchmarks"], [], "latest", False))
        assert rc_res == EXIT_OK
        assert res_out["target"] == "synology_nas"
        assert score_path.read_text(encoding="utf-8") == '{"acc": 1.0}'
        assert "benchmarks" not in load_evicted_manifest(data_dir)["evicted_datasets"]

        # Corrupting a pack byte on FTP is immediately caught by verify_snapshot
        packs_on_disk = list((ftp_root / "home" / "pydbsrt-backups" / "packs").glob("benchmarks_*.tar"))
        assert len(packs_on_disk) == 1
        packs_on_disk[0].write_bytes(packs_on_disk[0].read_bytes() + b"X")
        ver_corrupt = nas_tgt.verify_snapshot("latest", check_hashes=False)
        assert ver_corrupt["status"] == "corrupt"
    finally:
        server.shutdown()
        server.server_close()
