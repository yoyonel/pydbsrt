"""Hermetic unit and integration tests for scripts.data_access and pydbsrt-data-access skill."""

from __future__ import annotations

import io
import json
import struct
import time
from pathlib import Path
from unittest.mock import MagicMock, patch

from scripts.benchmarks.eval_skill_data_access import grade_response_text
from scripts.benchmarks.profile_skill_transcript import profile_transcript
from scripts.data_access.cli import main as cli_main
from scripts.data_access.constants import (
    CACHE_MAX_STALE_SECONDS,
    CACHE_SCHEMA_VERSION,
    CORP_VPN_HIJACK_CIDR,
    DBAHASH_HEADER_PREFIX_FORMAT,
    DBAHASH_MAGIC_PREFIX,
    DBPHASH_HEADER_PREFIX_FORMAT,
    DBPHASH_MAGIC_PREFIX,
    EXIT_OK,
    HOME_SUBNET_CIDR,
    NET_HOME_LAN,
    NET_NOMAD_WAN,
    NET_WORK_VPN,
)
from scripts.data_access.prober import (
    _atomic_write_cache,
    _evaluate_ahash_catalog_freshness,
    _evaluate_catalog_freshness,
    _evaluate_corpus_counts,
    _fetch_o2switch_https_snapshot,
    _parse_remote_snapshot,
    _read_dbahash_media_count,
    _read_dbphash_media_count,
    _try_load_valid_cache,
    probe_http_json_gateway,
    probe_local_cifs_safe,
)
from scripts.data_access.resolver import (
    build_or_run_fetch,
    inspect_family_container,
    normalize_family_id,
    normalize_release_stem,
    resolve_batch_specs,
    resolve_data_family,
    resolve_local_artifact_download,
)
from scripts.data_access.topology import (
    RuntimeEndpoints,
    WorkspaceContext,
    _classify_workstation_profile,
    seed_worktree_datasets,
)
from scripts.tooling.validate_skills import validate_single_skill


def _make_fake_workspace(tmp_path: Path, is_worktree: bool = False) -> WorkspaceContext:
    """Creates a synthetic WorkspaceContext backed by tmp_path."""
    common_repo = tmp_path / "parent_repo"
    active_repo = (tmp_path / "worktree_repo") if is_worktree else common_repo
    (common_repo / "data" / "snippets").mkdir(parents=True, exist_ok=True)
    (common_repo / "data" / "SRT").mkdir(parents=True, exist_ok=True)
    active_repo.mkdir(parents=True, exist_ok=True)
    return WorkspaceContext(
        active_repo=active_repo,
        common_repo=common_repo,
        is_worktree=is_worktree,
        effective_data_dir=common_repo / "data",
        unseeded_items=("data/snippets", "data/SRT/couplings.json") if is_worktree else (),
        env_vars={
            "MEDIA_MOUNT_ROOT": str(tmp_path / "mnt"),
            "NAS_HOST": "nas.example.test",
            "NAS_USER": "testuser",
            "O2SWITCH_URL": "https://portal.example.test/dbsrt",
        },
    )


def test_classify_home_lan_wins_over_corporate_vpn() -> None:
    """Home Wi-Fi with direct /24 route wins over ppp0 192.168.0.0/16 route."""
    conns = [{"name": "Freebox-3CA108", "type": "802-11-wireless", "device": "wlan0"}]
    ifaces = {"wlan0": ["10.0.0.2/24"], "ppp0": ["10.99.0.2/32"]}
    routes = [
        {"dst": HOME_SUBNET_CIDR, "dev": "wlan0"},
        {"dst": CORP_VPN_HIJACK_CIDR, "dev": "ppp0"},
    ]
    profile, cifs_safe, vpn_hijack = _classify_workstation_profile(conns, ifaces, routes)
    assert profile == NET_HOME_LAN
    assert cifs_safe is True
    assert vpn_hijack is True


def test_classify_work_vpn_blocks_cifs_stat() -> None:
    """Corporate Wi-Fi + ppp0 192.168.0.0/16 without direct /24 disables CIFS stat()."""
    conns = [{"name": "Letsignit Internal", "type": "802-11-wireless", "device": "wlan0"}]
    ifaces = {"wlan0": ["10.20.0.5/24"], "ppp0": ["10.99.0.2/32"]}
    routes = [{"dst": CORP_VPN_HIJACK_CIDR, "dev": "ppp0"}]
    profile, cifs_safe, vpn_hijack = _classify_workstation_profile(conns, ifaces, routes)
    assert profile == NET_WORK_VPN
    assert cifs_safe is False
    assert vpn_hijack is True


def test_classify_nomad_hotspot_blocks_cifs_stat() -> None:
    """4G/5G mobile hotspot disables CIFS stat() even if ppp0 is active."""
    conns = [{"name": "Redmi Note 13 Pro+ 5G", "type": "802-11-wireless", "device": "wlan0"}]
    ifaces = {"wlan0": ["172.20.10.2/24"], "ppp0": ["10.99.0.2/32"]}
    routes = [{"dst": CORP_VPN_HIJACK_CIDR, "dev": "ppp0"}]
    profile, cifs_safe, _ = _classify_workstation_profile(conns, ifaces, routes)
    assert profile == NET_NOMAD_WAN
    assert cifs_safe is False


def test_probe_local_cifs_never_calls_scandir_when_unsafe(tmp_path: Path) -> None:
    """Guarantees zero filesystem syscalls on CIFS paths when cifs_safe_to_stat is False."""
    endpoints = RuntimeEndpoints(
        local_media_root=str(tmp_path / "cifs_trap"),
        nas_lan_host="10.0.0.58",
        nas_wan_host="nas.example.test",
        nas_user="u",
        nas_pass_entry="",
        rpi4_lan_host="10.0.0.24",
        o2switch_url="https://portal.example.test/dbsrt",
    )
    with patch("os.scandir", side_effect=AssertionError("os.scandir must not be called!")) as mock_scan:
        res = probe_local_cifs_safe({"cifs_safe_to_stat": False}, endpoints)
    assert res["accessible"] is False
    assert res["error"] == "blocked_by_stale_cifs_hang_guard"
    mock_scan.assert_not_called()


def test_probe_http_gateway_rejects_html_200_and_accepts_json() -> None:
    """Prevents false-positive accessible=True when WAN :80 returns text/html instead of JSON."""
    fake_html_resp = MagicMock()
    fake_html_resp.status = 200
    fake_html_resp.headers.get_content_type.return_value = "text/html"
    fake_html_resp.read.return_value = b"<html>router page</html>"
    fake_html_resp.__enter__.return_value = fake_html_resp

    with patch("urllib.request.urlopen", return_value=fake_html_resp):
        bad = probe_http_json_gateway("WAN HTTP", "http://nas.example.test:80/health")
    assert bad["accessible"] is False
    assert "non_json_response" in bad["error"]

    fake_json_resp = MagicMock()
    fake_json_resp.status = 200
    fake_json_resp.headers.get_content_type.return_value = "application/json"
    fake_json_resp.read.return_value = json.dumps(
        {"status": "ok", "mounted_shares": ["Disque_1", "Disque_1_1", "Disque_2"]}
    ).encode("utf-8")
    fake_json_resp.__enter__.return_value = fake_json_resp

    with patch("urllib.request.urlopen", return_value=fake_json_resp):
        good = probe_http_json_gateway("LAN HTTP", "http://10.0.0.24:18089/health")
    assert good["accessible"] is True
    assert good["mounted_shares"] == ["Disque_1", "Disque_1_1", "Disque_2"]


def test_dbphash_header_and_stale_freshness_evaluation(tmp_path: Path) -> None:
    """Reads DBSRT v2 media_count from 16-byte header and detects STALE_LOCAL drift."""
    cat_file = tmp_path / "catalog.dbphash"
    header = struct.pack(DBPHASH_HEADER_PREFIX_FORMAT, DBPHASH_MAGIC_PREFIX + b"\x02\x00\x00", 2, 0, 3089)
    cat_file.write_bytes(header + b"\x00" * 112)
    assert _read_dbphash_media_count(cat_file) == 3089

    acat_file = tmp_path / "catalog.dbahash"
    aheader = struct.pack(DBAHASH_HEADER_PREFIX_FORMAT, DBAHASH_MAGIC_PREFIX, 1, 1, 32, 2490)
    acat_file.write_bytes(aheader + b"\x00" * 108)
    assert _read_dbahash_media_count(acat_file) == 2490

    snap = _parse_remote_snapshot(
        "o2switch",
        "cat=395529096:1794618913:4974\nacat=5503776992:1794689460:2490\nphash=4974:2504\nahash=2548:2548\ncouplings=41263950:1794262595\n",
    )
    assert snap["catalog_dbahash"]["media_count"] == 2490
    assert snap["catalog_dbahash"]["size_bytes"] == 5503776992
    nodes = {
        "local": {
            "catalog_dbphash": {
                "exists": True,
                "path": str(cat_file),
                "size_bytes": 241013560,
                "mtime_epoch": 1794432635,
                "media_count": 3089,
            },
            "catalog_dbahash": {
                "exists": True,
                "path": str(acat_file),
                "size_bytes": 5503776992,
                "mtime_epoch": 1794689460,
                "media_count": 2490,
            },
            "ahash_files": {"count": 538},
        },
        "rpi4": {
            "catalog_dbphash": dict(snap["catalog_dbphash"], path="rpi4:~/dbsrt/data/catalog.dbphash"),
            "catalog_dbahash": {"exists": False, "size_bytes": 0, "mtime_epoch": 0, "media_count": 0},
            "ahash_files": {"count": 0},
        },
        "o2switch": snap,
    }
    cat_eval = _evaluate_catalog_freshness(nodes, {"ssh_rpi4": {"accessible": True}})
    assert cat_eval["status"] == "STALE_LOCAL"
    assert cat_eval["delta_medias"] == 1885
    assert "rpi4:~/dbsrt/data/catalog.dbphash" in str(cat_eval["sync_cmd"])

    acat_eval = _evaluate_ahash_catalog_freshness(nodes, {"ssh_o2switch": {"accessible": True}})
    assert acat_eval["status"] == "UP_TO_DATE"
    assert acat_eval["authoritative_node"] == "o2switch"
    assert acat_eval["delta_medias"] == 0

    ahash_eval = _evaluate_corpus_counts(nodes, "ahash_files")
    assert ahash_eval["status"] == "PARTIAL_LOCAL"
    assert ahash_eval["most_complete_node"] == "o2switch"
    assert ahash_eval["most_complete_count"] == 2548
    assert ahash_eval["missing_locally"] == 2010


def test_resolve_freebox_three_disks_and_snippet_couplings(tmp_path: Path) -> None:
    """Verifies all 3 Freebox disks and normalized snippet-to-coupling matching."""
    ws = _make_fake_workspace(tmp_path, is_worktree=True)
    snip_mp4 = ws.effective_data_dir / "snippets" / "Andor.S01E03.1080p.WEB.h264-KOGi_mid15s.mp4"
    snip_mp4.write_bytes(b"fake_mp4")
    (ws.effective_data_dir / "snippets" / "Andor.S01E03.1080p.WEB.h264-KOGi_mid15s.phash").write_bytes(b"ph")

    couplings_payload = {
        "couplings": [
            {
                "media_name": "Andor.S01E03.1080p.WEB.h264-KOGi.mkv.24.0fps",
                "media_hash": "7c74374145e45be484c01c819af68ca4",
                "phash_filename": "Andor.S01E03.1080p.WEB.h264-KOGi.mkv.24.0fps.phash",
                "subtitles": [{"lang": "en"}, {"lang": "fr"}],
            }
        ]
    }
    (ws.effective_data_dir / "SRT" / "couplings.json").write_text(json.dumps(couplings_payload), encoding="utf-8")

    fake_status = {
        "workspace": {
            "effective_data_dir": str(ws.effective_data_dir),
            "unseeded_items": list(ws.unseeded_items),
        },
        "network": {"profile": NET_NOMAD_WAN, "cifs_safe_to_stat": False},
        "channels": {
            "local_fs": {"accessible": True, "latency_ms": 0.1},
            "local_cifs": {"accessible": False, "latency_ms": 0.0},
            "https_o2switch": {"accessible": True, "latency_ms": 120.0},
            "ssh_o2switch": {"accessible": False, "latency_ms": 3000.0},
        },
        "freshness": {
            "packed_catalog": {"status": "UP_TO_DATE"},
            "ahash_corpus": {"status": "UP_TO_DATE"},
        },
    }

    fb = resolve_data_family(ws, fake_status, "freebox")
    assert len(fb["freebox_disks"]) == 3
    disk_ids = [d["id"] for d in fb["freebox_disks"]]
    assert disk_ids == ["disque1", "disque2", "disque1_1"]
    assert fb["recommended_channel"]["id"] == "o2switch_https_bounce"

    snips = resolve_data_family(ws, fake_status, "snippets", query="Andor")
    matches = snips["query_result"]["matches"]
    assert len(matches) == 1
    assert matches[0]["parent_media_coupled"] is True
    assert matches[0]["parent_coupling"]["certified_subtitles_count"] == 2
    assert matches[0]["parent_coupling"]["languages"] == ["en", "fr"]

    fetch_no_item = build_or_run_fetch(ws, fake_status, "ahash", dry_run=True)
    assert fetch_no_item["ok"] is False
    assert fetch_no_item["error"] == "https_fallback_requires_item"

    fetch_https_dry = build_or_run_fetch(ws, fake_status, "ahash", item="002835455e305309.ahash", dry_run=True)
    assert fetch_https_dry["ok"] is True
    assert fetch_https_dry["source_node"] == "o2switch_https"
    assert "/api/artifact/download?kind=ahash&file=002835455e305309.ahash" in fetch_https_dry["url"]

    fetch_cat_blocked = build_or_run_fetch(ws, fake_status, "catalog", dry_run=True)
    assert fetch_cat_blocked["ok"] is False
    assert fetch_cat_blocked["error"] == "no_ssh_channel_reachable"


def test_seed_worktree_and_cache_invalidation(tmp_path: Path) -> None:
    """Tests worktree dataset symlinking and /proc/net/route cache invalidation."""
    ws = _make_fake_workspace(tmp_path, is_worktree=True)
    (ws.common_repo / "data" / "catalog.dbahash").write_bytes(b"DBAHASH\x00")
    (ws.common_repo / "data" / "SRT" / "couplings.json").write_text("{}", encoding="utf-8")

    preview = seed_worktree_datasets(ws, dry_run=True)
    assert preview["status"] == "dry_run"
    assert "data/snippets" in preview["created"]
    assert "data/catalog.dbahash" in preview["created"]
    assert not (ws.active_repo / "data" / "snippets").exists()

    applied = seed_worktree_datasets(ws, dry_run=False)
    assert applied["status"] == "seeded"
    assert (ws.active_repo / "data" / "snippets").is_symlink()
    assert (ws.active_repo / "data" / "catalog.dbahash").is_symlink()
    assert (ws.active_repo / "data" / "SRT" / "couplings.json").is_symlink()

    cache_file = tmp_path / "cache.json"
    now = time.time()
    _atomic_write_cache(
        cache_file,
        {
            "schema_version": CACHE_SCHEMA_VERSION,
            "created_at_epoch": now - 10.0,
            "route_digest": "digest_a",
            "workspace": {"active_repo": str(ws.active_repo)},
        },
    )
    fresh_hit = _try_load_valid_cache(cache_file, "digest_a", str(ws.active_repo), CACHE_MAX_STALE_SECONDS)
    assert fresh_hit is not None
    assert fresh_hit["cache"]["stale_revalidating"] is False
    assert _try_load_valid_cache(cache_file, "digest_b", str(ws.active_repo), CACHE_MAX_STALE_SECONDS) is None

    _atomic_write_cache(
        cache_file,
        {
            "schema_version": CACHE_SCHEMA_VERSION,
            "created_at_epoch": now - 600.0,
            "route_digest": "digest_a",
            "workspace": {"active_repo": str(ws.active_repo)},
        },
    )
    swr_hit = _try_load_valid_cache(cache_file, "digest_a", str(ws.active_repo), CACHE_MAX_STALE_SECONDS)
    assert swr_hit is not None
    assert swr_hit["cache"]["stale_revalidating"] is True


def test_batch_query_token_boundary_and_deterministic_evaluator(tmp_path: Path) -> None:
    """Verifies single-shot batch query, token-boundary snippet filtering, and evaluator regexes."""
    ws = _make_fake_workspace(tmp_path, is_worktree=False)
    (ws.effective_data_dir / "snippets" / "Andor.S01E01.1080p.WEB.h264-KOGi_mid15s.mp4").write_bytes(b"mp4")
    (ws.effective_data_dir / "snippets" / "Skins_S03E02_Pandora_mid15s.mp4").write_bytes(b"mp4")
    (ws.effective_data_dir / "SRT" / "couplings.json").write_text('{"couplings": []}', encoding="utf-8")

    fake_status = {
        "cache": {"hit": True},
        "workspace": {"effective_data_dir": str(ws.effective_data_dir), "unseeded_items": []},
        "network": {"host_env": "workstation", "profile": NET_HOME_LAN, "cifs_safe_to_stat": True},
        "channels": {"local_fs": {"accessible": True, "latency_ms": 0.1}},
        "freshness": {"packed_catalog": {"status": "UP_TO_DATE"}, "ahash_corpus": {"status": "UP_TO_DATE"}},
    }
    batched = resolve_batch_specs(ws, fake_status, ["freebox", "snippets:Andor"])
    assert "freebox" in batched["results"]
    matches = batched["results"]["snippets:Andor"]["query_result"]["matches"]
    assert [m["filename"] for m in matches] == ["Andor.S01E01.1080p.WEB.h264-KOGi_mid15s.mp4"]

    sample_report = (
        "local: 538 | rpi4: 0 | o2switch: 2 548. "
        "Non, pas à jour (STALE_LOCAL): local 241 013 560 octets (2026-11-11) vs 395 529 096 (2026-11-14). "
        "Freebox: Disque_1, Disque_2, Disque_1_1 (/mnt/freebox/Disque_1_1). "
        "Andor.S01E01.1080p.WEB.h264-KOGi_mid15s.mp4 -> 5cc955f9c40a26756418d6aa198ea273 (21 sous-titres); "
        "S01E05 et S01E06 non couplés."
    )
    graded = grade_response_text(sample_report)
    assert graded["all_passed"] is True
    assert graded["score"] == "16/16"


def test_cli_simulated_profile_and_skill_validator() -> None:
    """Verifies CLI --simulate-profile execution, <8.5KB query ceiling, and workspace skill validation gate."""
    assert normalize_family_id("catalog.dbphash") == "packed-catalog"
    assert normalize_family_id("catalog.dbahash") == "packed-ahash-catalog"
    assert normalize_family_id("dbahash") == "packed-ahash-catalog"
    assert normalize_release_stem("Andor.S01E03.1080p.WEB.h264-KOGi_mid15s.mp4") == "andor.s01e03.1080p.web.h264-kogi"

    buf = io.StringIO()
    with patch("sys.stdout", buf):
        rc = cli_main(["status", "--simulate-profile", "work_vpn", "--compact"])
    assert rc == EXIT_OK
    parsed = json.loads(buf.getvalue())
    assert parsed["network"]["profile"] == "work_vpn"
    assert parsed["network"]["cifs_safe_to_stat"] is False
    assert "packed_ahash_catalog" in parsed["freshness"]

    q_buf = io.StringIO()
    with patch("sys.stdout", q_buf):
        q_rc = cli_main(["query", "--simulate-profile", "work_vpn", "freebox", "dbahash", "catalog"])
    assert q_rc == EXIT_OK
    raw_q_out = q_buf.getvalue()
    assert len(raw_q_out.encode("utf-8")) < 8500
    assert max(len(line) for line in raw_q_out.splitlines()) < 5000

    repo_root = Path(__file__).resolve().parents[2]
    skill_dir = repo_root / ".agents" / "skills" / "pydbsrt-data-access"
    assert validate_single_skill(skill_dir, repo_root) == []


def test_profile_skill_transcript_detects_anomalies_and_clean_run(tmp_path: Path) -> None:
    """Verifies generic second-by-second profiler and automatic bottleneck flags."""
    noisy_path = tmp_path / "conv_noisy" / ".system_generated" / "logs" / "transcript_full.jsonl"
    noisy_path.parent.mkdir(parents=True, exist_ok=True)
    noisy_steps = [
        {"step_index": 0, "type": "SYSTEM_MESSAGE", "created_at": "2026-11-14T13:00:00Z", "content": "prompt"},
        {
            "step_index": 1,
            "type": "PLANNER_RESPONSE",
            "created_at": "2026-11-14T13:00:05Z",
            "input_tokens": 1000,
            "output_tokens": 500,
            "thinking": "x" * 12000,
            "tool_calls": [{"name": "read_file", "args": {"Path": "/tmp/output.txt"}}],
        },
        {
            "step_index": 2,
            "type": "GENERIC",
            "created_at": "2026-11-14T13:00:08Z",
            "content": "Created At: 2026-11-14T13:00:05Z\nCompleted At: 2026-11-14T13:00:07Z\n[full output: rtk/tee/a.log]",
        },
    ]
    noisy_path.write_text("\n".join(json.dumps(s) for s in noisy_steps), encoding="utf-8")
    prof = profile_transcript(noisy_path, include_steps=False)
    assert prof["wall_clock_seconds"] == 8.0
    assert prof["tool_execution_seconds"] == 2.0
    assert prof["llm_inference_and_writing_seconds"] == 6.0
    assert "OUTPUT_TXT_SPILL_10KB" in prof["detected_anomalies"]
    assert "RTK_TEE_TRUNCATION" in prof["detected_anomalies"]
    assert any(a.startswith("THINKING_LOOP_SPIKE:") for a in prof["detected_anomalies"])


def test_https_artifact_fallback_and_traversal_guard(tmp_path: Path) -> None:
    """Verifies traversal-proof artifact resolution (including NFC/NFD & ellipsis) and HTTPS fallback."""
    import hashlib
    import unicodedata

    data_root = tmp_path / "srv_data"
    (data_root / "__AHASH__").mkdir(parents=True)
    (data_root / "__PHASH__").mkdir(parents=True)
    (data_root / "SRT").mkdir(parents=True)
    raw_bytes = b"DBSRTAH1" + b"\xab\xcd" * 64
    expected_sha = hashlib.sha256(raw_bytes).hexdigest()
    (data_root / "__AHASH__" / "sample_ep01.ahash").write_bytes(raw_bytes)
    nfd_name = unicodedata.normalize("NFD", "01 Cruauté.mp4.86.1fps.ahash")
    nfc_query = unicodedata.normalize("NFC", "01 Cruauté.mp4.86.1fps.ahash")
    (data_root / "__AHASH__" / nfd_name).write_bytes(raw_bytes)
    ellipsis_name = "Live #28  - On fait le Bilan, calmement....mp4.30.0fps.phash"
    (data_root / "__PHASH__" / ellipsis_name).write_bytes(raw_bytes)
    (data_root / "SRT" / "couplings.json").write_text('{"couplings":[]}', encoding="utf-8")

    ok_path, reason, code = resolve_local_artifact_download(data_root, "ahash", "sample_ep01.ahash")
    assert code == 200 and reason == "ok" and ok_path is not None
    nfc_hit, _, nfc_code = resolve_local_artifact_download(data_root, "ahash", nfc_query)
    assert nfc_code == 200 and nfc_hit is not None and nfc_hit.name == nfd_name
    ell_hit, _, ell_code = resolve_local_artifact_download(data_root, "phash", ellipsis_name)
    assert ell_code == 200 and ell_hit is not None
    assert resolve_local_artifact_download(data_root, "ahash", "../SRT/couplings.json")[2] == 400
    assert resolve_local_artifact_download(data_root, "ahash", "..")[2] == 400
    assert resolve_local_artifact_download(data_root, "ahash", "bad\x00name.ahash")[2] == 400
    assert resolve_local_artifact_download(data_root, "phash", "sample_ep01.ahash")[2] == 400
    assert resolve_local_artifact_download(data_root, "unknown", "sample_ep01.ahash")[2] == 400
    assert resolve_local_artifact_download(data_root, "ahash", "missing.ahash")[2] == 404
    assert resolve_local_artifact_download(data_root, "couplings", "")[2] == 200

    ws = _make_fake_workspace(tmp_path / "client", is_worktree=False)
    ws.env_vars["DBSRT_API_KEY"] = "unit-test-key"
    nomad_status = {
        "channels": {
            "ssh_rpi4": {"accessible": False},
            "ssh_o2switch": {"accessible": False},
            "https_o2switch": {"accessible": True},
        }
    }

    good_resp = MagicMock()
    good_resp.headers = {"X-Checksum-SHA256": expected_sha}
    good_resp.read.side_effect = [raw_bytes, b""]
    good_resp.__enter__.return_value = good_resp
    dest_file = tmp_path / "downloaded" / "sample_ep01.ahash"

    with patch("urllib.request.urlopen", return_value=good_resp):
        res = build_or_run_fetch(ws, nomad_status, "ahash", item="sample_ep01.ahash", dest_override=str(dest_file))
    assert res["ok"] is True
    assert res["source_node"] == "o2switch_https"
    assert res["sha256"] == expected_sha
    assert dest_file.read_bytes() == raw_bytes

    bad_resp = MagicMock()
    bad_resp.headers = {"X-Checksum-SHA256": "0" * 64}
    bad_resp.read.side_effect = [raw_bytes, b""]
    bad_resp.__enter__.return_value = bad_resp
    bad_dest = tmp_path / "downloaded" / "tampered.ahash"

    with patch("urllib.request.urlopen", return_value=bad_resp):
        bad_res = build_or_run_fetch(ws, nomad_status, "ahash", item="sample_ep01.ahash", dest_override=str(bad_dest))
    assert bad_res["ok"] is False
    assert "sha256_mismatch" in bad_res["error"]
    assert not bad_dest.exists()


def test_ahash_and_phash_human_title_sidecar_search(tmp_path: Path, monkeypatch: object) -> None:
    """Verifies accent-folded human-title sidecar lookup (.ahash/.phash) and canonical moviehash priority."""
    monkeypatch.setenv("DBSRT_DATA_ACCESS_CACHE", str(tmp_path / "cache" / "status.json"))  # type: ignore[attr-defined]
    ws = _make_fake_workspace(tmp_path, is_worktree=False)
    ah_dir = ws.effective_data_dir / "__AHASH__"
    ph_dir = ws.effective_data_dir / "__PHASH__"
    ah_dir.mkdir(parents=True, exist_ok=True)
    ph_dir.mkdir(parents=True, exist_ok=True)

    (ah_dir / "01f6f860fd5d63ed.86.1fps.ahash").write_bytes(b"ahash_bytes")
    (ah_dir / "01f6f860fd5d63ed.json").write_text(
        json.dumps(
            {
                "schema_version": 1,
                "moviehash": "01f6f860fd5d63ed",
                "media_name": "Ep.04 | Hôpital, la république en panne.mp4",
                "ahash_file": "01f6f860fd5d63ed.86.1fps.ahash",
                "fps": 86.125,
                "frames": 212099,
                "duration_sec": 2462.688,
                "selected_audio_stream": {"codec_name": "aac", "language": "fre"},
            }
        ),
        encoding="utf-8",
    )

    (ph_dir / "01f6f860fd5d63ed.24.0fps.phash").write_bytes(b"phash_hex")
    (ph_dir / "01f6f860fd5d63ed.json").write_text(
        json.dumps(
            {
                "moviehash": "01f6f860fd5d63ed",
                "media_name": "Ep.04 | Hôpital, la république en panne.mp4",
                "fps": 24.0,
                "frames": 59104,
            }
        ),
        encoding="utf-8",
    )
    (ph_dir / "%5B%20OxTorrent.cc%20%5D%20Les.101.Dalmatiens.1961.mkv.24.0fps.phash").write_bytes(b"ph_url")
    (ws.effective_data_dir / "SRT" / "couplings.json").write_text('{"couplings": []}', encoding="utf-8")

    fake_status = {
        "cache": {"hit": True},
        "workspace": {"effective_data_dir": str(ws.effective_data_dir), "unseeded_items": []},
        "network": {"host_env": "workstation", "profile": NET_HOME_LAN, "cifs_safe_to_stat": True},
        "channels": {"local_fs": {"accessible": True, "latency_ms": 0.1}},
        "freshness": {"packed_catalog": {"status": "UP_TO_DATE"}, "ahash_corpus": {"status": "UP_TO_DATE"}},
    }

    batched = resolve_batch_specs(ws, fake_status, ["ahash:Hopital panne", "phash:101 Dalmatiens"])
    ah_matches = batched["results"]["ahash:Hopital panne"]["query_result"]["matches"]
    assert len(ah_matches) == 1
    assert ah_matches[0]["ahash_file"] == "01f6f860fd5d63ed.86.1fps.ahash"
    assert ah_matches[0]["moviehash"] == "01f6f860fd5d63ed"
    assert ah_matches[0]["duration_sec"] == 2462.69
    assert ah_matches[0]["phash_file"] == "01f6f860fd5d63ed.24.0fps.phash"

    ph_matches = batched["results"]["phash:101 Dalmatiens"]["query_result"]["matches"]
    assert len(ph_matches) == 1
    assert "Dalmatiens" in ph_matches[0]["media_name"]


def _mock_json_response(payload: dict[str, object]) -> MagicMock:
    """Creates a context-manager HTTP 200 application/json mock response."""
    resp = MagicMock()
    resp.status = 200
    resp.headers.get_content_type.return_value = "application/json"
    resp.read.return_value = json.dumps(payload).encode("utf-8")
    resp.__enter__.return_value = resp
    return resp


def test_multi_network_media_search_and_inspect_zero_cifs_on_nomad_and_vpn(tmp_path: Path, monkeypatch: object) -> None:
    """Proves zero CIFS scandir on nomad_wan/work_vpn while resolving nested TV episodes & remote ffprobe."""
    monkeypatch.setenv("DBSRT_DATA_ACCESS_CACHE", str(tmp_path / "cache" / "status.json"))  # type: ignore[attr-defined]
    ws = _make_fake_workspace(tmp_path, is_worktree=False)
    ws.env_vars["DBSRT_API_KEY"] = "test-api-key"
    (ws.effective_data_dir / "SRT" / "couplings.json").write_text('{"couplings": []}', encoding="utf-8")

    for profile in (NET_NOMAD_WAN, NET_WORK_VPN):
        status = {
            "cache": {"hit": True},
            "workspace": {"effective_data_dir": str(ws.effective_data_dir), "unseeded_items": []},
            "network": {"host_env": "workstation", "profile": profile, "cifs_safe_to_stat": False},
            "channels": {
                "local_fs": {"accessible": True},
                "local_cifs": {"accessible": False},
                "https_o2switch": {"accessible": True, "latency_ms": 110.0},
                "wan_ftp_synology": {"accessible": True, "latency_ms": 300.0},
            },
            "freshness": {"packed_catalog": {"status": "UP_TO_DATE"}, "ahash_corpus": {"status": "UP_TO_DATE"}},
        }

        def fake_urlopen(req: object, timeout: float = 5.0) -> MagicMock:
            url = getattr(req, "full_url", str(req))
            if "source=freebox-disks" in url and "/api/files" in url:
                return _mock_json_response(
                    {
                        "status": "ok",
                        "total_pages": 1,
                        "items": [
                            {
                                "name": "Boite Noire (2021)[1080p].mkv",
                                "path": "Disque_1_1/Vidéos/Boite Noire (2021)[1080p].mkv",
                                "is_dir": False,
                                "size_bytes": 9387647743,
                            }
                        ],
                    }
                )
            if "source=synology-tvshow" in url and "subdir=" in url:
                if "subdir=Locke" in url and "S01" not in url:
                    return _mock_json_response(
                        {
                            "status": "ok",
                            "total_pages": 1,
                            "items": [{"name": "S01", "path": "Locke & Key/S01", "is_dir": True, "size_bytes": 0}],
                        }
                    )
                if "S01" in url:
                    return _mock_json_response(
                        {
                            "status": "ok",
                            "total_pages": 1,
                            "items": [
                                {
                                    "name": "Locke.and.Key.S01E01.1080p.mkv",
                                    "path": "Locke & Key/S01/Locke.and.Key.S01E01.1080p.mkv",
                                    "is_dir": False,
                                    "size_bytes": 2455763846,
                                }
                            ],
                        }
                    )
                return _mock_json_response(
                    {
                        "status": "ok",
                        "total_pages": 1,
                        "items": [{"name": "Locke & Key", "path": "Locke & Key", "is_dir": True, "size_bytes": 0}],
                    }
                )
            if "/api/file/details" in url:
                many_subs = [
                    {"index": idx + 2, "codec": "subrip", "language": "fre" if idx == 12 else f"l{idx:02d}"}
                    for idx in range(20)
                ]
                return _mock_json_response(
                    {
                        "status": "ok",
                        "probed": True,
                        "size_bytes": 9387647743,
                        "duration_seconds": 7783.79,
                        "format_name": "matroska,webm",
                        "bit_rate": 9648405,
                        "video": {"codec": "h264", "width": 1920, "height": 800, "fps": "24/1"},
                        "audio_streams": [{"index": 1, "codec": "dts", "channels": 6, "language": "fre"}],
                        "subtitle_streams": many_subs,
                    }
                )
            raise AssertionError(f"Unexpected URL: {url}")

        real_scandir = __import__("os").scandir

        def guarded_scandir(
            target_dir: str | Path,
            prof: str = profile,
            scan_fn: object = real_scandir,
        ) -> object:
            if str(target_dir).startswith(str(tmp_path / "mnt")):
                raise AssertionError(f"Forbidden CIFS scandir on {target_dir} under {prof}!")
            return scan_fn(target_dir)  # type: ignore[operator]

        with (
            patch("os.scandir", side_effect=guarded_scandir),
            patch("urllib.request.urlopen", side_effect=fake_urlopen),
        ):
            batched = resolve_batch_specs(ws, status, ["freebox:Boite Noire", "tvshow:Locke and Key S01E01"])
            fb_hits = batched["results"]["freebox:Boite Noire"]["query_result"]["matches"]
            assert len(fb_hits) == 1
            assert fb_hits[0]["disk"] == "Disque_1_1"
            assert "local_cifs_path" not in fb_hits[0]

            tv_hits = batched["results"]["tvshow:Locke and Key S01E01"]["query_result"]["matches"]
            assert len(tv_hits) == 1
            assert tv_hits[0]["path"] == "Locke & Key/S01/Locke.and.Key.S01E01.1080p.mkv"

            insp = inspect_family_container(ws, status, "freebox", "Boite Noire")
            assert insp["ok"] is True
            assert insp["via"] == "o2switch_https_api"
            assert insp["video"]["width"] == 1920
            assert insp["subtitle_streams_count"] == 20
            assert insp["subtitle_streams_truncated"] is True
            assert insp["subtitle_streams"][0]["language"] == "fre"


def test_o2switch_https_freshness_snapshot_when_ssh_blocked() -> None:
    """Verifies HTTPS :443 /api/phashes?limit=1 fallback populates remote catalog & phash counts."""
    resp = _mock_json_response(
        {"status": "ok", "summary": {"raw_count": 4974, "catalog_count": 4974, "catalog_size_bytes": 395529096}}
    )
    with patch("urllib.request.urlopen", return_value=resp):
        snap = _fetch_o2switch_https_snapshot("https://portal.example.test/dbsrt", "secret-key")
    assert snap is not None
    assert snap["transport"] == "https_api"
    assert snap["catalog_dbphash"]["media_count"] == 4974
    assert snap["phash_files"]["count"] == 4974
