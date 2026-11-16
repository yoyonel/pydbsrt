"""Comprehensive unit, functional, and integration tests for src/pydbsrt/portal/."""

from __future__ import annotations

import http.server
import json
import runpy
import subprocess
import sys
import threading
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any
from unittest.mock import MagicMock

import pytest

from pydbsrt.portal import cache as portal_cache
from pydbsrt.portal import cli as portal_cli
from pydbsrt.portal import config as portal_config
from pydbsrt.portal import server as portal_server
from pydbsrt.portal.models import AssetsQueryCriteria
from pydbsrt.portal.services import (
    assets_service,
    ffprobe_service,
    ftp_parsers,
    listing_service,
    phash_microscopy,
    phash_reconciliation,
    probe_service,
    secrets_env,
    trajectory_service,
)


class _FakeFTP:
    """Configurable in-memory FTP mock covering MLSD, LIST, NLST, and CWD modes."""

    def __init__(
        self,
        *,
        mlsd_entries: list[tuple[str, dict[str, str]]] | None = None,
        list_lines: list[str] | None = None,
        nlst_entries: list[str] | None = None,
        fail_feat: bool = False,
        fail_list: bool = False,
        fail_cwd_prefixes: tuple[str, ...] = (),
    ) -> None:
        self.mlsd_entries = mlsd_entries or []
        self.list_lines = list_lines or []
        self.nlst_entries = nlst_entries or []
        self.fail_feat = fail_feat
        self.fail_list = fail_list
        self.fail_cwd_prefixes = fail_cwd_prefixes
        self.cwd_history: list[str] = []

    def connect(self, _host: str, _port: int, timeout: float = 5.0) -> None:
        return None

    def login(self, _user: str, _passwd: str) -> None:
        return None

    def cwd(self, path: str) -> None:
        if any(path.startswith(p) for p in self.fail_cwd_prefixes):
            raise OSError(f"CWD denied: {path}")
        self.cwd_history.append(path)

    def sendcmd(self, cmd: str) -> str:
        if cmd == "FEAT" and self.fail_feat:
            raise OSError("FEAT unsupported")
        return "211-Features:\n MLSD\n211 End"

    def mlsd(self) -> list[tuple[str, dict[str, str]]]:
        return list(self.mlsd_entries)

    def retrlines(self, _cmd: str, callback: Any) -> None:
        if self.fail_list:
            raise OSError("LIST unsupported")
        for line in self.list_lines:
            callback(line)

    def nlst(self, _path: str = "") -> list[str]:
        return list(self.nlst_entries)

    def quit(self) -> None:
        return None


def test_ftp_parsers_mlsd_list_nlst_and_directory_fallback() -> None:
    """Verifies MLSD, UNIX LIST, NLST parsers and three-tier FTP listing fallback."""
    dir_item = ftp_parsers._parse_mlsd_item("Series", {"type": "dir", "modify": "20261116123045"}, "video")
    assert dir_item is not None and dir_item["is_dir"] is True
    assert dir_item["modify"] == "2026-11-16 12:30:45"

    vid_item = ftp_parsers._parse_mlsd_item("Dune.2021.1080p.mkv", {"type": "file", "size": "2048"}, "")
    assert vid_item is not None and vid_item["is_dir"] is False and vid_item["size_bytes"] == 2048

    assert ftp_parsers._parse_mlsd_item("notes.txt", {"type": "file", "size": "10"}, "") is None
    assert ftp_parsers._parse_nlst_item("Season 01", "Show") is not None
    assert ftp_parsers._parse_nlst_item("Episode.mp4", "Show") is not None

    assert (
        ftp_parsers._parse_ftp_list_line("drwxr-xr-x 2 root root 4096 Nov 16 12:00 Andor", "")["is_dir"]  # type: ignore[index]
        is True
    )
    file_line = ftp_parsers._parse_ftp_list_line("-rw-r--r-- 1 root root 4096 Nov 16 12:00 Andor.mkv", "sub")
    assert file_line is not None and file_line["path"] == "sub/Andor.mkv"
    assert ftp_parsers._parse_ftp_list_line("-rw-r--r-- 1 root root 10 Nov 16 12:00 readme.nfo", "") is None
    assert ftp_parsers._parse_ftp_list_line("-rw-r--r-- 1 root root 10 Nov 16 12:00 .hidden", "") is None
    assert ftp_parsers._parse_ftp_list_line("malformed line", "") is None

    # 1. MLSD primary
    ftp_mlsd = _FakeFTP(mlsd_entries=[("Film.2024.mkv", {"type": "file", "size": "100"}), (".hidden", {"type": "dir"})])
    res_mlsd = ftp_parsers._parse_ftp_directory_items(ftp_mlsd, "")  # type: ignore[arg-type]
    assert len(res_mlsd) == 1 and res_mlsd[0]["name"] == "Film.2024.mkv"

    # 2. LIST fallback when FEAT fails
    ftp_list = _FakeFTP(
        fail_feat=True,
        list_lines=["drwxr-xr-x 2 root root 4096 Nov 16 12:00 Season 1", "invalid"],
    )
    res_list = ftp_parsers._parse_ftp_directory_items(ftp_list, "")  # type: ignore[arg-type]
    assert len(res_list) == 1 and res_list[0]["name"] == "Season 1"

    # 3. NLST fallback when FEAT and LIST fail
    ftp_nlst = _FakeFTP(fail_feat=True, fail_list=True, nlst_entries=["Blade.Runner.mkv", "__PHASH__"])
    res_nlst = ftp_parsers._parse_ftp_directory_items(ftp_nlst, "SciFi")  # type: ignore[arg-type]
    assert len(res_nlst) == 1 and res_nlst[0]["path"] == "SciFi/Blade.Runner.mkv"


def test_secrets_env_resolution_and_topology(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    """Verifies secret lookup hierarchy, host environment detection, couplings, and CIFS guard."""
    monkeypatch.setattr(secrets_env, "_SECRET_MEMORY_CACHE", {})
    monkeypatch.setenv("NAS_PASS", "env-secret-123")
    assert secrets_env.resolve_nas_password() == "env-secret-123"
    # Second call hits memory cache
    monkeypatch.delenv("NAS_PASS")
    assert secrets_env.resolve_nas_password() == "env-secret-123"

    # Secret file discovery via ~/dbsrt/etc/
    fake_home = tmp_path / "home"
    etc_dir = fake_home / "dbsrt" / "etc"
    etc_dir.mkdir(parents=True)
    (etc_dir / ".freebox_pass").write_text("file-fbx-secret\nsecond-line", encoding="utf-8")
    monkeypatch.setattr(Path, "home", lambda: fake_home)
    monkeypatch.delenv("FREEBOX_PASS", raising=False)
    assert secrets_env.resolve_freebox_password() == "file-fbx-secret"

    # Pass subprocess fallback
    monkeypatch.setattr(secrets_env, "_SECRET_MEMORY_CACHE", {})
    monkeypatch.setattr(
        secrets_env.subprocess,
        "run",
        lambda *_a, **_kw: subprocess.CompletedProcess([], 0, stdout="pass-secret\n", stderr=""),
    )
    assert secrets_env._lookup_secret_uncached("MISSING_VAR", ".missing_file", "entry/x") == "pass-secret"

    # Environment detection
    monkeypatch.setattr(secrets_env.socket, "gethostname", lambda: "sapin.o2switch.net")
    assert secrets_env.detect_host_environment() == "o2switch"
    monkeypatch.setattr(secrets_env.socket, "gethostname", lambda: "raspberrypi4")
    assert secrets_env.detect_host_environment() == "rpi4"
    monkeypatch.setattr(secrets_env.socket, "gethostname", lambda: "workstation-dev")
    assert secrets_env.detect_host_environment() == "local"

    # Coupling lookup
    srt_dir = tmp_path / "data" / "SRT"
    srt_dir.mkdir(parents=True)
    (srt_dir / "couplings.json").write_text(
        json.dumps(
            {
                "couplings": [
                    {
                        "media_name": "Dune (2021) [2160p].mkv",
                        "fps": 24.0,
                        "frame_count": 1000,
                        "phash_filename": "abc.phash",
                        "subtitles": [{"lang": "fr"}],
                    }
                ]
            }
        ),
        encoding="utf-8",
    )
    monkeypatch.chdir(tmp_path)
    hit = secrets_env._lookup_coupling("Dune (2021) [2160p].mkv")
    assert hit is not None and hit["subtitles_count"] == 1
    assert secrets_env._lookup_coupling("NonExistentFilm.mkv") is None

    # CIFS safety cache + Freebox disk fallback on OSError
    monkeypatch.setattr(secrets_env, "_CIFS_SAFETY_CACHE", None)
    monkeypatch.setattr(secrets_env, "inspect_network_topology", lambda: {"cifs_safe_to_stat": True})
    assert secrets_env._is_cifs_safe_to_stat() is True
    monkeypatch.setattr(secrets_env, "load_raw_registry", lambda: (_ for _ in ()).throw(OSError("offline")))
    assert len(secrets_env._get_freebox_disk_specs()) == 3
    assert secrets_env._resolve_freebox_cifs_mount("rpi4_disque2") is not None
    assert secrets_env._resolve_freebox_cifs_mount("invalid_disk") is None


def test_ffprobe_service_endpoints_commands_and_execution(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    """Verifies endpoint resolution, ffprobe command builder, stream parsing, and error handling."""
    monkeypatch.setattr(portal_cache, "_DETAILS_CACHE", {})
    monkeypatch.setenv("DBSRT_PORTAL_CACHE_DIR", str(tmp_path / "cache"))
    monkeypatch.setattr(secrets_env, "resolve_nas_password", lambda: "nas-pw")
    monkeypatch.setattr(secrets_env, "resolve_freebox_password", lambda: "fbx-pw")

    assert ffprobe_service._resolve_target_endpoint("synology-video", "cifs", "Film.mkv")[0] is True
    assert ffprobe_service._resolve_target_endpoint("synology-tvshow", "wan_ftp", "Show/E01.mkv")[1] is True
    assert ffprobe_service._resolve_target_endpoint("synology-video", "unknown", "x")[0:2] == (False, False)
    assert "mafreebox" in ffprobe_service._resolve_freebox_endpoint("lan_ftp", "Disque 1/x.mkv")[3]
    assert ffprobe_service._resolve_freebox_endpoint("wan_http", "Disque_1/x y.mkv")[3].startswith("http://")
    assert ffprobe_service._resolve_freebox_endpoint("lan_http", "Disque_1/x.mkv")[3].startswith("http://")
    assert ffprobe_service._resolve_freebox_endpoint("rpi4_gateway", "Disque_1/x.mkv")[3].startswith("http://127.0.0.1")
    assert ffprobe_service._resolve_freebox_endpoint("unknown", "x")[3] == ""
    assert ffprobe_service._resolve_target_endpoint("unknown-src", "local", "x")[0:2] == (False, False)

    # ffprobe binary fallback when shutil.which returns None
    fake_home = tmp_path / "home"
    bin_dir = fake_home / "dbsrt" / "bin"
    bin_dir.mkdir(parents=True)
    fake_ffprobe = bin_dir / "ffprobe"
    fake_ffprobe.write_text("#!/bin/sh\n", encoding="utf-8")
    fake_ffprobe.chmod(0o755)
    monkeypatch.setattr(Path, "home", lambda: fake_home)
    monkeypatch.setattr(ffprobe_service.shutil, "which", lambda _n: None)
    assert ffprobe_service._resolve_ffprobe_binary() == str(fake_ffprobe)

    # Command builder across local, FTP, HTTP, and invalid
    sample_vid = tmp_path / "movie.mkv"
    sample_vid.write_bytes(b"\x00" * 32)
    assert (
        sample_vid.name in ffprobe_service._build_ffprobe_command(True, False, str(sample_vid), "", "", "ffprobe")[-1]
    )
    ftp_cmd = ffprobe_service._build_ffprobe_command(
        False, True, "", "ftp://mafreebox.freebox.fr:21/a.mkv", "pw", "ffprobe"
    )
    assert "-ftp-user" in ftp_cmd and "freebox" in ftp_cmd
    http_cmd = ffprobe_service._build_ffprobe_command(False, False, "", "http://127.0.0.1/a.mkv", "", "ffprobe")
    assert http_cmd[-1].startswith("http://")
    assert ffprobe_service._build_ffprobe_command(False, False, "", "", "", "ffprobe") == []

    # Execution outcomes: success, 550 permission error, generic error, timeout, missing target
    sample_probe_json = {
        "format": {"duration": "125.0", "size": "2097152", "bit_rate": "5000000", "format_name": "matroska"},
        "streams": [
            {"codec_type": "video", "codec_name": "hevc", "width": 1920, "height": 1080, "r_frame_rate": "24/1"},
            {"codec_type": "audio", "index": 1, "codec_name": "aac", "channels": 2, "tags": {"language": "fre"}},
            {
                "codec_type": "subtitle",
                "index": 2,
                "codec_name": "subrip",
                "tags": {"language": "fre"},
                "disposition": {"forced": 1, "default": 1},
            },
        ],
    }
    monkeypatch.setattr(
        ffprobe_service.subprocess,
        "run",
        lambda *_a, **_kw: subprocess.CompletedProcess([], 0, stdout=json.dumps(sample_probe_json), stderr=""),
    )
    snip_dir = tmp_path / "data" / "snippets"
    snip_dir.mkdir(parents=True)
    (snip_dir / "movie.mkv").write_bytes(b"\x00" * 64)
    monkeypatch.chdir(tmp_path)
    details = ffprobe_service.inspect_file_details("local-snippets", "local", "movie.mkv")
    assert details["probed"] is True
    assert details["video"]["codec"] == "hevc"
    assert len(details["audio_streams"]) == 1
    assert details["subtitle_streams"][0]["forced"] is True
    # Cached hit
    assert ffprobe_service.inspect_file_details("local-snippets", "local", "movie.mkv")["probed"] is True

    monkeypatch.setattr(
        ffprobe_service.subprocess,
        "run",
        lambda *_a, **_kw: subprocess.CompletedProcess([], 1, stdout="", stderr="550 Permission denied"),
    )
    ok_res, err_550 = ffprobe_service._execute_ffprobe(True, False, str(sample_vid), "", "", "ffprobe")
    assert ok_res is None and "550" in err_550

    monkeypatch.setattr(
        ffprobe_service.subprocess,
        "run",
        lambda *_a, **_kw: subprocess.CompletedProcess([], 1, stdout="", stderr="Invalid data"),
    )
    assert "Invalid data" in ffprobe_service._execute_ffprobe(True, False, str(sample_vid), "", "", "ffprobe")[1]

    def _raise_timeout(*_a: Any, **_kw: Any) -> Any:
        raise subprocess.TimeoutExpired(cmd="ffprobe", timeout=12.0)

    monkeypatch.setattr(ffprobe_service.subprocess, "run", _raise_timeout)
    assert "Timeout" in ffprobe_service._execute_ffprobe(True, False, str(sample_vid), "", "", "ffprobe")[1]

    monkeypatch.setattr(ffprobe_service, "_resolve_ffprobe_binary", lambda: None)
    no_bin = ffprobe_service.inspect_file_details("local-snippets", "local", "movie.mkv", force=True)
    assert no_bin["probed"] is False and "ffprobe non disponible" in no_bin["reason"]


def test_probe_and_listing_services_cifs_ftp_and_sort(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    """Verifies local FS edge states, FTP probes/listings, gateway fallback, and all sort/filter modes."""
    monkeypatch.setattr(portal_cache, "_LISTING_CACHE", {})
    monkeypatch.setenv("DBSRT_PORTAL_CACHE_DIR", str(tmp_path / "cache"))

    # Local FS missing / empty / unmounted / online / scandir error
    assert probe_service.probe_local_fs_profile("Missing", str(tmp_path / "nope"))["status"] == "missing"
    empty_dir = tmp_path / "empty"
    empty_dir.mkdir()
    assert probe_service.probe_local_fs_profile("Empty", str(empty_dir))["status"] == "unmounted"
    txt_dir = tmp_path / "txt_only"
    txt_dir.mkdir()
    (txt_dir / "notes.txt").write_text("hi", encoding="utf-8")
    assert probe_service.probe_local_fs_profile("TxtOnly", str(txt_dir))["status"] == "empty"

    monkeypatch.setattr(probe_service.os, "scandir", lambda _p: (_ for _ in ()).throw(OSError("Permission denied")))
    assert probe_service.probe_local_fs_profile("Err", str(txt_dir))["status"] == "error"
    monkeypatch.undo()
    monkeypatch.setattr(portal_cache, "_LISTING_CACHE", {})
    monkeypatch.setenv("DBSRT_PORTAL_CACHE_DIR", str(tmp_path / "cache"))

    # FTP probe & Freebox FTP probe
    monkeypatch.setattr(
        probe_service.ftplib,
        "FTP",
        lambda **_kw: _FakeFTP(
            nlst_entries=["FilmA.mkv", "__PHASH__"],
            mlsd_entries=[("FilmB.mkv", {"type": "file", "size": "123"})],
            fail_cwd_prefixes=("/Disque 2",),
        ),
    )
    p_ftp = probe_service.probe_ftp_profile("NAS FTP", "nas.local", "user", "pw", "/video")
    assert p_ftp["accessible"] is True and p_ftp["files_count"] == 1

    p_fbx = probe_service.probe_freebox_ftp_profile("FBX FTP", "mafreebox.freebox.fr", "freebox", "pw")
    assert p_fbx["accessible"] is True and p_fbx["files_count"] == 2

    def _raise_ftp(**_kw: Any) -> Any:
        raise OSError("timeout")

    monkeypatch.setattr(probe_service.ftplib, "FTP", _raise_ftp)
    p_fbx_err = probe_service.probe_freebox_ftp_profile("FBX FTP", "mafreebox.freebox.fr", "freebox", "pw")
    assert p_fbx_err["status"] == "offline" and "Hôte LAN inaccessible" in p_fbx_err["reason"]
    assert probe_service.probe_ftp_profile("NAS FTP", "mafreebox.freebox.fr", "u", "p", "/")["status"] == "offline"

    # Gateway probe network error
    monkeypatch.setattr(
        probe_service.urllib.request, "urlopen", lambda *_a, **_kw: (_ for _ in ()).throw(OSError("refused"))
    )
    assert probe_service.probe_http_gateway_profile("GW", "http://127.0.0.1:1/health")["status"] == "offline"

    # execute_single_probe branches
    monkeypatch.setattr(secrets_env, "resolve_nas_password", lambda: "pw")
    monkeypatch.setattr(secrets_env, "_is_cifs_safe_to_stat", lambda: False)
    assert probe_service.execute_single_probe("synology-video", "cifs")["status"] == "offline"
    assert probe_service.execute_single_probe("synology-tvshow", "wan_ftp")["status"] == "offline"
    assert probe_service.execute_single_probe("freebox-disks", "disque1")["status"] == "offline"
    assert probe_service.execute_single_probe("freebox-disks", "lan_ftp")["status"] == "offline"
    assert probe_service.execute_single_probe("freebox-disks", "rpi4_gateway")["status"] == "offline"

    # Listing CIFS, Synology FTP, Freebox FTP, and sorting/filtering
    monkeypatch.setattr(
        listing_service.ftplib,
        "FTP",
        lambda **_kw: _FakeFTP(mlsd_entries=[("Andor.S01E01.2022.1080p.x265.mkv", {"type": "file", "size": "999"})]),
    )
    res_syn_ftp = listing_service.list_source_files("synology-tvshow", "lan_ftp", subdir="Andor", force=True)
    assert len(res_syn_ftp["items"]) == 1

    res_fbx_root = listing_service.list_source_files("freebox-disks", "lan_ftp", subdir="", force=True)
    assert len(res_fbx_root["items"]) == 3
    res_fbx_bad = listing_service.list_source_files("freebox-disks", "lan_ftp", subdir="Photos/priv", force=True)
    assert res_fbx_bad["items"] == []
    res_fbx_ok = listing_service.list_source_files(
        "freebox-disks", "lan_ftp", subdir="Disque 1/Téléchargements", force=True
    )
    assert len(res_fbx_ok["items"]) == 1

    # Local directory scan + multi-token search + all sort modes
    vid_dir = tmp_path / "data" / "snippets"
    vid_dir.mkdir(parents=True)
    (vid_dir / "Sub").mkdir()
    (vid_dir / "Alpha.2020.1080p.x264.mkv").write_bytes(b"\x00" * 100)
    (vid_dir / "Beta.2022.2160p.hevc.mp4").write_bytes(b"\x00" * 200)
    monkeypatch.chdir(tmp_path)
    for sort_mode in ("name_asc", "name_desc", "size_desc", "size_asc", "date_desc"):
        listed = listing_service.list_source_files(
            "local-snippets", "local", query_str="2022 2160p hevc", sort_mode=sort_mode, force=True
        )
        assert listed["filtered_items"] == 1
    # Memory cache hit on second call
    cached_list = listing_service.list_source_files("local-snippets", "local", force=False)
    assert cached_list["status"] == "ok"
    assert listing_service._fetch_cifs_items("freebox-disks", "invalid", "")[1] == "unknown"


def test_assets_service_cli_fallback_and_psycopg2_mocks(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    """Verifies PostgreSQL CLI fallback, psycopg2 error/not_found paths, and physical/pHash asset checks."""
    # Force CLI fallback branch (_connect_pg -> None)
    monkeypatch.setattr(assets_service, "_connect_pg", lambda: None)
    monkeypatch.setattr(assets_service, "_query_pg_cli", lambda _sql: None)
    err_res = assets_service.get_pg_assets(page=1, limit=5)
    assert err_res["status"] == "error" and err_res["connected"] is False
    assert assets_service.get_pg_asset_detail(42)["status"] == "error"

    fake_row = {
        "id": 42,
        "name": "Dune.2021.mkv",
        "clean_title": "Dune",
        "season": None,
        "episode": None,
        "year": 2021,
        "resolution": "2160p",
        "video_codec": "hevc",
        "fps": 24.0,
        "frame_count": 100,
        "expected_frames": 100,
        "is_vfr": False,
        "duration_ms": 4000,
        "file_size_bytes": 2048,
        "moviehash": "002835455e305309",
        "media_hash": "hash",
        "video_rel_path": "/video/Dune.2021.mkv",
        "source_id": "synology-video",
        "phash_status": "complete",
    }

    def _fake_pg_cli(sql: str) -> str | None:
        if "json_build_object" in sql and "FROM medias WHERE id" in sql:
            return json.dumps(fake_row)
        if "json_build_object" in sql:
            return json.dumps({"total": 1, "nominal": 1, "missing_phash": 0, "unmapped_path": 0})
        if "SELECT count(*)" in sql:
            return "1"
        if "SHOW server_version" in sql:
            return "17.2 (Debian 17.2)"
        if "json_agg" in sql:
            return json.dumps([fake_row])
        return None

    monkeypatch.setattr(assets_service, "_query_pg_cli", _fake_pg_cli)
    ok_cli = assets_service.get_pg_assets(
        page=3, limit=10, status_filter="nominal", source_filter="synology-video", search_q="Dune", sort_by="title_asc"
    )
    assert ok_cli["status"] == "ok" and ok_cli["page"] == 1 and ok_cli["pg_version"] == "PostgreSQL 17.2"
    detail_cli = assets_service.get_pg_asset_detail(42)
    assert detail_cli["status"] == "ok" and len(detail_cli["alt_channels"]) == 1

    # Mock psycopg2 connection for detail not_found & error handling
    mock_cur = MagicMock()
    mock_cur.fetchone.return_value = None
    mock_conn = MagicMock()
    mock_conn.cursor.return_value = mock_cur
    assert assets_service._get_pg_asset_detail_psycopg2(mock_conn, 999)["status"] == "not_found"

    mock_conn_err = MagicMock()
    mock_conn_err.cursor.side_effect = RuntimeError("db boom")
    assert assets_service._get_pg_assets_psycopg2(mock_conn_err, AssetsQueryCriteria())["status"] == "error"

    # Physical & pHash checks + status tags
    assert assets_service._compute_asset_status_tag("pending", "/v.mkv") == "missing_phash"
    assert assets_service._compute_asset_status_tag("complete", "") == "unmapped_path"
    assert assets_service._resolve_physical_file_exists("local-snippets", "foo.mp4")[0] is False
    assert assets_service._resolve_physical_file_exists("synology-tvshow", "tvshow/Show/E01.mkv")[0] is False
    assert assets_service._check_phash_file_exists(None, 24.0) is False

    # _query_pg_cli with mocked .pg_pass & subprocess
    fake_home = tmp_path / "h"
    etc = fake_home / "dbsrt" / "etc"
    etc.mkdir(parents=True)
    (etc / ".pg_pass").write_text("pgpass\n", encoding="utf-8")
    monkeypatch.setattr(Path, "home", lambda: fake_home)
    monkeypatch.undo()
    monkeypatch.setattr(Path, "home", lambda: fake_home)
    monkeypatch.setattr(assets_service.subprocess, "check_output", lambda *_a, **_kw: b"17.1\n")
    assert assets_service._query_pg_cli("SHOW server_version;") == "17.1"


def test_phash_and_trajectory_services_edge_cases(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    """Verifies pHash edge cases, trajectory sorting, sidecar title resolution, and on-demand runs."""
    # pHash invalid header / empty file / cut sampling > 200
    bad_cat = tmp_path / "bad.dbphash"
    bad_cat.write_bytes(b"NOPE" + b"\x00" * 124)
    assert phash_reconciliation._read_catalog_dbphash(bad_cat) == ({}, [])

    empty_ph = tmp_path / "data" / "__PHASH__"
    empty_ph.mkdir(parents=True)
    (empty_ph / "0000000000000000.24.0fps.phash").write_bytes(b"")
    monkeypatch.chdir(tmp_path)
    assert phash_microscopy.inspect_phash_microscopic("")["status"] == "error"
    assert phash_microscopy.inspect_phash_microscopic("0000000000000000")["status"] == "error"

    # Cut sampling > MICROSCOPIC_CUTS_SAMPLE_LIMIT (alternating 0x0 and 0xFFFFFFFFFFFFFFFF -> 64-bit delta)
    hashes = tuple(0 if i % 2 == 0 else (1 << 64) - 1 for i in range(250))
    sampled_cuts, total_cuts = phash_microscopy._detect_cuts(hashes, 25.0)
    assert total_cuts == 249 and len(sampled_cuts) == portal_config.MICROSCOPIC_CUTS_SAMPLE_LIMIT

    # Trajectory sidecar title resolution + status + sorting
    monkeypatch.setattr(trajectory_service, "_PHASH_SIDECAR_TITLE_CACHE", {})
    (empty_ph / "abcdef0123456789.json").write_text(json.dumps({"media_name": "Arrival (2016).mkv"}), encoding="utf-8")
    assert trajectory_service._resolve_phash_target_title("abcdef0123456789.24.0fps.phash", "") == "Arrival (2016).mkv"
    assert trajectory_service._resolve_phash_target_title("abcdef0123456789", "") == "Arrival (2016).mkv"
    assert trajectory_service._resolve_phash_target_title("PlainTitle.mkv", "") == "PlainTitle.mkv"

    assert trajectory_service._matches_trajectory_status(
        {"consensus_media_id": 1, "is_temporally_coherent": True}, "coherent"
    )
    assert trajectory_service._matches_trajectory_status({"consensus_media_id": 1}, "matched")
    assert trajectory_service._matches_trajectory_status({"consensus_media_id": None}, "unmatched")
    assert trajectory_service._matches_trajectory_status(
        {"consensus_media_id": 1, "is_temporally_coherent": False}, "divergent"
    )

    sample_recs = [
        {"target_media": "B", "duration_sec": 20.0, "total_elapsed_seconds": 1.0, "matched_cuts_count": 2},
        {"target_media": "A", "duration_sec": 10.0, "total_elapsed_seconds": 3.0, "matched_cuts_count": 5},
    ]
    for mode in (
        "name_asc",
        "name_desc",
        "duration_desc",
        "duration_asc",
        "elapsed_desc",
        "elapsed_asc",
        "matched_desc",
        "matched_asc",
    ):
        assert len(trajectory_service._sort_trajectory_records(sample_recs, mode)) == 2

    # run_on_demand_trajectory: missing, timeout, nonzero, success
    assert trajectory_service.run_on_demand_trajectory(str(tmp_path / "missing.mp4"))["status"] == "error"
    vid_file = tmp_path / "clip.mp4"
    vid_file.write_bytes(b"\x00" * 16)

    def _raise_traj_timeout(*_a: Any, **_kw: Any) -> Any:
        raise subprocess.TimeoutExpired(cmd="runner", timeout=60.0)

    monkeypatch.setattr(trajectory_service.subprocess, "run", _raise_traj_timeout)
    assert "timed out" in trajectory_service.run_on_demand_trajectory(str(vid_file))["error"]

    monkeypatch.setattr(
        trajectory_service.subprocess,
        "run",
        lambda *_a, **_kw: subprocess.CompletedProcess([], 1, stdout="", stderr="boom"),
    )
    assert trajectory_service.run_on_demand_trajectory(str(vid_file))["error"] == "boom"

    rep_dir = tmp_path / "data" / "reports"
    rep_dir.mkdir(parents=True)
    (rep_dir / "trajectory_batch_results.jsonl").write_text('{"target_media":"clip.mp4","cuts":[]}\n', encoding="utf-8")
    monkeypatch.setattr(
        trajectory_service.subprocess,
        "run",
        lambda *_a, **_kw: subprocess.CompletedProcess([], 0, stdout="ok", stderr=""),
    )
    ok_traj = trajectory_service.run_on_demand_trajectory(str(vid_file))
    assert ok_traj["status"] == "ok" and ok_traj["item"]["target_media"] == "clip.mp4"


def test_http_server_routes_cli_and_main_module(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    """Integration test covering HEAD, favicon, POST /api/trajectory/run, 404, CLI flags, and __main__."""
    # 1. HTTP Server edge routes
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), portal_server.PortalRequestHandler)
    port = server.server_port
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    base_url = f"http://127.0.0.1:{port}"
    try:
        req_head = urllib.request.Request(f"{base_url}/sources.html", method="HEAD")
        with urllib.request.urlopen(req_head, timeout=5) as resp:
            assert resp.status == 200

        with urllib.request.urlopen(f"{base_url}/favicon.ico", timeout=5) as resp:
            assert resp.status == 200
            assert resp.headers.get("Content-Type") == "image/svg+xml"

        req_bad_post = urllib.request.Request(f"{base_url}/api/unknown", data=b"{}", method="POST")
        with pytest.raises(urllib.error.HTTPError) as exc_404:
            urllib.request.urlopen(req_bad_post, timeout=5)
        assert exc_404.value.code == 404

        req_traj_err = urllib.request.Request(f"{base_url}/api/trajectory/run", data=b"not-json", method="POST")
        with pytest.raises(urllib.error.HTTPError) as exc_500:
            urllib.request.urlopen(req_traj_err, timeout=5)
        assert exc_500.value.code == 500

        monkeypatch.setattr(
            "pydbsrt.portal.routes.trajectories.run_on_demand_trajectory",
            lambda _t, cuts=5, frames=20: {"status": "ok", "cuts": cuts, "frames": frames},
        )
        req_traj_ok = urllib.request.Request(
            f"{base_url}/api/trajectory/run",
            data=json.dumps({"target": "x.mp4", "cuts": 3, "frames": 10}).encode(),
            method="POST",
        )
        with urllib.request.urlopen(req_traj_ok, timeout=5) as resp_ok:
            assert resp_ok.status == 200
            assert json.loads(resp_ok.read().decode())["cuts"] == 3

        monkeypatch.setattr(
            "pydbsrt.portal.routes.assets.get_pg_asset_detail",
            lambda _mid: {"status": "not_found", "message": "absent"},
        )
        with pytest.raises(urllib.error.HTTPError) as exc_asset_404:
            urllib.request.urlopen(f"{base_url}/api/asset/details?id=999", timeout=5)
        assert exc_asset_404.value.code == 404
    finally:
        server.shutdown()
        server.server_close()

    # 2. run_server graceful KeyboardInterrupt
    fake_httpd = MagicMock()
    fake_httpd.__enter__.return_value = fake_httpd
    fake_httpd.serve_forever.side_effect = KeyboardInterrupt
    monkeypatch.setattr(portal_server, "ReusableThreadingTCPServer", lambda *_a, **_kw: fake_httpd)
    portal_server.run_server(8789)
    fake_httpd.serve_forever.assert_called_once()

    # 3. CLI dispatch across all flags + __main__ entrypoint
    monkeypatch.setattr(portal_cli, "run_server", lambda _p: None)
    monkeypatch.setattr(portal_cli, "get_sources_metadata", lambda: {"status": "ok"})
    monkeypatch.setattr(portal_cli, "execute_single_probe", lambda s, p: {"source": s, "profile": p})
    monkeypatch.setattr(portal_cli, "list_source_files", lambda *_a, **_kw: {"items": []})
    monkeypatch.setattr(portal_cli, "inspect_file_details", lambda *_a, **_kw: {"probed": False})
    monkeypatch.setattr(portal_cli, "get_pg_assets", lambda **_kw: {"items": []})
    monkeypatch.setattr(portal_cli, "get_pg_asset_detail", lambda mid: {"id": mid})
    monkeypatch.setattr(portal_cli, "query_phashes_list", lambda _c: {"items": []})
    monkeypatch.setattr(portal_cli, "inspect_phash_microscopic", lambda m, **_kw: {"moviehash": m})
    monkeypatch.setattr(portal_cli, "list_trajectory_audit_files", lambda: [])
    monkeypatch.setattr(portal_cli, "query_trajectory_records", lambda _c: {"items": []})
    monkeypatch.setattr(portal_cli, "get_trajectory_record_detail", lambda _f, t: {"target": t})

    for argv in (
        ["--metadata"],
        ["--probe", "local-snippets", "local"],
        ["--files", "local-snippets", "local"],
        ["--details", "local-snippets", "local", "clip.mp4"],
        ["--assets"],
        ["--asset-id", "7"],
        ["--phashes"],
        ["--phash-moviehash", "002835455e305309"],
        ["--trajectory-files"],
        ["--trajectories"],
        ["--trajectory-details", "clip.mp4"],
        ["--port", "8781"],
    ):
        portal_cli.main(argv)

    monkeypatch.setattr(sys, "argv", ["pydbsrt.portal", "--metadata"])
    runpy.run_module("pydbsrt.portal", run_name="__main__")
