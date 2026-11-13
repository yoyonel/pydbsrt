"""Unit tests for `scripts/ingestion/backfill_ahash_sidecars.py`."""

from __future__ import annotations

import json
from pathlib import Path

from scripts.ingestion import backfill_ahash_sidecars as bf


def test_parse_storage_string_never_touches_filesystem():
    """String parsing resolves Synology & Freebox paths without touching unmounted CIFS paths."""
    syno_tv = bf.parse_storage_string("/media/cifs/Synology/tvshow/Show/Ep1.mkv")
    assert syno_tv == bf.SourceLocation("synology_tvshow", "tvshow/Show/Ep1.mkv", "ftp")

    syno_vid = bf.parse_storage_string("/media/cifs/Synology/video/Film (2021).mkv")
    assert syno_vid == bf.SourceLocation("synology_video", "video/Film (2021).mkv", "ftp")

    freebox = bf.parse_storage_string("/media/cifs/Freebox/Disque_1/Anime/Ep2.mkv")
    assert freebox == bf.SourceLocation("freebox_disks", "Disque_1/Anime/Ep2.mkv", "http")


def test_build_stream_url_and_child_env_keep_credentials_out_of_argv(tmp_path: Path):
    cfg = bf.SourceEnvConfig(
        nas_host="nas.example.invalid",
        nas_user="nas_reader",
        nas_pass_entry="",
        explicit_nas_pass="s3cr3t_pass",
        freebox_base="http://freebox.example.invalid:18089",
    )
    ftp_url, ftp_err = bf.build_stream_url(
        bf.SourceLocation("synology_tvshow", "tvshow/My Show/Ep 1.mkv", "ftp"),
        cfg,
    )
    assert ftp_err is None
    assert ftp_url == "ftp://nas.example.invalid/tvshow/My Show/Ep 1.mkv"
    assert "nas_reader" not in ftp_url
    assert "s3cr3t_pass" not in ftp_url

    http_url, http_err = bf.build_stream_url(
        bf.SourceLocation("freebox_disks", "Disque_1/My Movie [2021].mkv", "http"),
        cfg,
    )
    assert http_err is None
    assert http_url == "http://freebox.example.invalid:18089/Disque_1/My%20Movie%20%5B2021%5D.mkv"

    env = bf.build_child_env(cfg, "s3cr3t_pass")
    assert env["DBSRT_FTP_USER"] == "nas_reader"
    assert env["DBSRT_FTP_PASSWORD"] == "s3cr3t_pass"
    assert env["FTP_USER"] == "nas_reader"
    assert env["FTP_PASSWORD"] == "s3cr3t_pass"


def test_plan_target_item_idempotence_and_upgrade(tmp_path: Path):
    ahash_dir = tmp_path / "ahash"
    phash_dir = tmp_path / "phash"
    reports_dir = tmp_path / "reports"
    ahash_dir.mkdir()
    phash_dir.mkdir()
    reports_dir.mkdir()

    # 1. Legacy 16-hex moviehash .ahash resolved via __PHASH__/<hash>.json & nas_ftp_index
    h_missing = "0123456789abcdef"
    p_missing = ahash_dir / f"{h_missing}.86.1fps.ahash"
    p_missing.write_bytes(b"AHASH")
    (phash_dir / f"{h_missing}.json").write_text(json.dumps({"media_name": "MissingSidecar.mkv"}), encoding="utf-8")

    # 2. Complete single_track sidecar -> skipped without --force (preserves language & audio_index)
    h_done = "fedcba9876543210"
    p_done = ahash_dir / f"{h_done}.86.1fps.ahash"
    p_done.write_bytes(b"AHASH")
    (ahash_dir / f"{h_done}.json").write_text(
        json.dumps(
            {
                "media_name": "Done.mkv",
                "selection": "single_track",
                "selected_audio_stream": {"audio_index": 0, "language": "jpn"},
                "container_audio_streams": [{}],
            }
        ),
        encoding="utf-8",
    )

    # 3. Multi-track ffmpeg_default -> eligible for upgrade to verified_backfill
    p_upgrade = ahash_dir / "Upgrade.mkv.86.1fps.ahash"
    p_upgrade.write_bytes(b"AHASH")
    (ahash_dir / "Upgrade.mkv.json").write_text(
        json.dumps(
            {
                "media_name": "Upgrade.mkv",
                "selection": "ffmpeg_default",
                "container_audio_streams": [{"audio_index": 0}, {"audio_index": 1}],
            }
        ),
        encoding="utf-8",
    )

    (reports_dir / "nas_ftp_index.json").write_text(
        json.dumps(
            {
                "index": {
                    "MissingSidecar.mkv": "video/MissingSidecar.mkv",
                    "Upgrade.mkv": "tvshow/Show/Upgrade.mkv",
                }
            }
        ),
        encoding="utf-8",
    )
    cfg = bf.SourceEnvConfig("nas.invalid", "u", "", "p", "")
    idx = bf.build_source_index(reports_dir, cfg)

    plan_missing = bf.plan_target_item(p_missing, phash_dir, idx)
    assert plan_missing.moviehash == h_missing
    assert plan_missing.media_name == "MissingSidecar.mkv"
    assert plan_missing.skip_reason is None
    assert plan_missing.location == bf.SourceLocation("synology_video", "video/MissingSidecar.mkv", "ftp")

    plan_done = bf.plan_target_item(p_done, phash_dir, idx)
    assert plan_done.skip_reason == "already_certain_or_verified"
    assert plan_done.existing_language == "jpn"
    assert plan_done.existing_audio_index == 0
    skipped_rec = bf._make_record(plan_done, "skipped")
    assert skipped_rec.language == "jpn"
    assert skipped_rec.audio_index == 0

    plan_upgrade = bf.plan_target_item(p_upgrade, phash_dir, idx)
    assert plan_upgrade.skip_reason is None
    assert plan_upgrade.location == bf.SourceLocation("synology_tvshow", "tvshow/Show/Upgrade.mkv", "ftp")


def test_clamp_workers_and_dry_run_main(tmp_path: Path, monkeypatch):
    assert bf.clamp_workers(0) == 1
    assert bf.clamp_workers(3) == 3
    assert bf.clamp_workers(99) == bf.MAX_BACKFILL_WORKERS

    ahash_dir = tmp_path / "ahash"
    phash_dir = tmp_path / "phash"
    reports_dir = tmp_path / "reports"
    ahash_dir.mkdir()
    phash_dir.mkdir()
    reports_dir.mkdir()

    (ahash_dir / "Sample.mkv.86.1fps.ahash").write_bytes(b"AHASH")
    (reports_dir / "nas_ftp_index.json").write_text(
        json.dumps({"index": {"Sample.mkv": "tvshow/Sample.mkv"}}), encoding="utf-8"
    )
    report_out = reports_dir / "out.jsonl"
    monkeypatch.setenv("NAS_HOST", "nas.invalid")

    rc = bf.main(
        [
            "--ahash-dir",
            str(ahash_dir),
            "--phash-dir",
            str(phash_dir),
            "--reports-dir",
            str(reports_dir),
            "--report-out",
            str(report_out),
            "--dry-run",
        ]
    )
    assert rc == 0
    rows = [json.loads(line) for line in report_out.read_text(encoding="utf-8").splitlines()]
    assert len(rows) == 1
    assert rows[0]["status"] == "dry_run_ready"
    assert rows[0]["source_id"] == "synology_tvshow"
    assert rows[0]["source_rel_path"] == "tvshow/Sample.mkv"


def test_sha256_twin_inheritance_without_phash_sidecar(tmp_path: Path, monkeypatch):
    """A 16-hex .ahash without __PHASH__/<hash>.json inherits media identity/source from an exact SHA-256 twin."""
    ahash_dir = tmp_path / "ahash"
    phash_dir = tmp_path / "phash"
    reports_dir = tmp_path / "reports"
    ahash_dir.mkdir()
    phash_dir.mkdir()
    reports_dir.mkdir()

    twin_bytes = b"AHASH_EXACT_TWIN_BYTES_0123456789"
    (ahash_dir / "Exact.Twin.Show.mkv.86.1fps.ahash").write_bytes(twin_bytes)
    (ahash_dir / "4ba3e3f2c751f6f4.86.1fps.ahash").write_bytes(twin_bytes)
    (reports_dir / "nas_ftp_index.json").write_text(
        json.dumps({"index": {"Exact.Twin.Show.mkv": "tvshow/Exact.Twin.Show.mkv"}}),
        encoding="utf-8",
    )

    report_out = reports_dir / "twin_out.jsonl"
    monkeypatch.setenv("NAS_HOST", "nas.invalid")

    rc = bf.main(
        [
            "--ahash-dir",
            str(ahash_dir),
            "--phash-dir",
            str(phash_dir),
            "--reports-dir",
            str(reports_dir),
            "--report-out",
            str(report_out),
            "--dry-run",
        ]
    )
    assert rc == 0
    rows = {
        r["ahash_file"]: r for r in (json.loads(line) for line in report_out.read_text(encoding="utf-8").splitlines())
    }
    hex_row = rows["4ba3e3f2c751f6f4.86.1fps.ahash"]
    assert hex_row["status"] == "dry_run_ready"
    assert hex_row["moviehash"] == "4ba3e3f2c751f6f4"
    assert hex_row["media_name"] == "Exact.Twin.Show.mkv"
    assert hex_row["source_id"] == "synology_tvshow"
    assert hex_row["source_rel_path"] == "tvshow/Exact.Twin.Show.mkv"


def test_progress_line_and_summary_breakdowns(tmp_path: Path, capsys):
    records = [
        bf.BackfillRecord(
            ahash_file="a.86.1fps.ahash",
            sidecar_file="a.json",
            logical_stem="a",
            media_name="a.mkv",
            moviehash=None,
            source_id="synology_tvshow",
            source_rel_path="tvshow/a.mkv",
            status="written",
            selection="single_track",
            audio_index=0,
            language="jpn",
            track_count=1,
            elapsed_sec=0.91,
        ),
        bf.BackfillRecord(
            ahash_file="b.86.1fps.ahash",
            sidecar_file="b.json",
            logical_stem="b",
            media_name="b.mkv",
            moviehash=None,
            source_id="synology_tvshow",
            source_rel_path="tvshow/b.mkv",
            status="written",
            selection="verified_backfill",
            audio_index=0,
            language="fre",
            track_count=2,
            verified_distance=0.4425,
            elapsed_sec=3.39,
        ),
    ]
    assert bf._format_progress_line(1, 2, records[0]) == "[1/2] written (single_track, jpn, 0.91s) -> a.json"
    assert (
        bf._format_progress_line(2, 2, records[1]) == "[2/2] written (verified_backfill, fre, d=0.44, 3.39s) -> b.json"
    )
    bf._print_summary(records, tmp_path / "r.jsonl", dry_run=False)
    out = capsys.readouterr().out
    assert "By selection:" in out
    assert "single_track" in out
    assert "verified_backfill" in out
    assert "By language:" in out
    assert "jpn" in out
    assert "fre" in out


def _sample_sidecar(
    media_name: str,
    ahash_file: str,
    *,
    selection: str = "single_track",
    moviehash: str | None = None,
    source_rel_path: str | None = None,
) -> dict:
    return {
        "schema_version": 1,
        "moviehash": moviehash,
        "media_name": media_name,
        "ahash_file": ahash_file,
        "fps": 86.125,
        "frames": 100,
        "duration_sec": 1.16,
        "header": {
            "version": 1,
            "sample_rate": 5512,
            "frame_size": 2048,
            "hop_size": 64,
            "num_bands": 33,
            "spectral_lag": 2,
            "frame_count": 100,
        },
        "selection": selection,
        "selected_audio_stream": {
            "index": 1,
            "audio_index": 0,
            "codec_name": "aac",
            "channels": 2,
            "sample_rate": 48000,
            "language": "fre",
            "title": None,
            "is_default": True,
        },
        "container_audio_streams": [{"index": 1, "audio_index": 0}],
        "extracted_at": 1700000000,
        "source_id": "synology_tvshow" if source_rel_path else None,
        "source_rel_path": source_rel_path,
    }


def test_migrate_to_moviehash_phash_dedup_local_and_ftp_mocked(tmp_path: Path, monkeypatch):
    ahash_dir = tmp_path / "ahash"
    phash_dir = tmp_path / "phash"
    reports_dir = tmp_path / "reports"
    ahash_dir.mkdir()
    phash_dir.mkdir()
    reports_dir.mkdir()

    # 1. Resolved via __PHASH__ reverse index -> renamed cleanly
    h_ph = "1111222233334444"
    (phash_dir / f"{h_ph}.json").write_text(
        json.dumps({"moviehash": h_ph, "media_name": "Épisode [01].mkv"}), encoding="utf-8"
    )
    (ahash_dir / "Épisode [01].mkv.86.1fps.ahash").write_bytes(b"AHASH_PH")
    (ahash_dir / "Épisode [01].mkv.json").write_text(
        json.dumps(_sample_sidecar("Épisode [01].mkv", "Épisode [01].mkv.86.1fps.ahash")),
        encoding="utf-8",
    )

    # 2. Binary twin collision with existing <16hex> -> dedup + sidecar merge keeping verified_backfill + source_rel_path
    h_dup = "aaaa1111bbbb2222"
    (phash_dir / f"{h_dup}.json").write_text(
        json.dumps({"moviehash": h_dup, "media_name": "Twin.Show.mkv"}), encoding="utf-8"
    )
    (ahash_dir / f"{h_dup}.86.1fps.ahash").write_bytes(b"AHASH_IDENTICAL_TWIN")
    (ahash_dir / f"{h_dup}.json").write_text(
        json.dumps(
            _sample_sidecar("Twin.Show.mkv", f"{h_dup}.86.1fps.ahash", selection="ffmpeg_default", moviehash=h_dup)
        ),
        encoding="utf-8",
    )
    (ahash_dir / "Twin.Show.mkv.86.1fps.ahash").write_bytes(b"AHASH_IDENTICAL_TWIN")
    (ahash_dir / "Twin.Show.mkv.json").write_text(
        json.dumps(
            _sample_sidecar(
                "Twin.Show.mkv",
                "Twin.Show.mkv.86.1fps.ahash",
                selection="verified_backfill",
                source_rel_path="tvshow/Twin.Show.mkv",
            )
        ),
        encoding="utf-8",
    )

    # 3. Resolved via local file MovieHash (no __PHASH__ entry)
    local_media = tmp_path / "Local.Sample.mkv"
    local_bytes = bytes(i % 251 for i in range(200_000))
    local_media.write_bytes(local_bytes)
    h_local = "e19d5212c9812cd6"
    (ahash_dir / "Local.Sample.mkv.86.1fps.ahash").write_bytes(b"AHASH_LOCAL")
    local_sidecar = _sample_sidecar("Local.Sample.mkv", "Local.Sample.mkv.86.1fps.ahash")
    local_sidecar["source_id"] = "local_fixture"
    local_sidecar["source_rel_path"] = str(local_media)
    (ahash_dir / "Local.Sample.mkv.json").write_text(json.dumps(local_sidecar), encoding="utf-8")

    # 4. Resolved via mocked remote FTP MovieHash (no __PHASH__ entry)
    h_ftp = "fedcba0987654321"
    (ahash_dir / "Remote.Ftp.mkv.86.1fps.ahash").write_bytes(b"AHASH_FTP")
    (ahash_dir / "Remote.Ftp.mkv.json").write_text(
        json.dumps(
            _sample_sidecar("Remote.Ftp.mkv", "Remote.Ftp.mkv.86.1fps.ahash", source_rel_path="tvshow/Remote.Ftp.mkv")
        ),
        encoding="utf-8",
    )

    monkeypatch.setenv("NAS_HOST", "nas.invalid")
    monkeypatch.setenv("NAS_USER", "u")
    monkeypatch.setenv("NAS_PASS", "p")
    monkeypatch.setattr(bf._worker, "target_moviehash", lambda target, base, env: h_ftp)

    dry_report = reports_dir / "mig_dry.jsonl"
    rc_dry = bf.main(
        [
            "--ahash-dir",
            str(ahash_dir),
            "--phash-dir",
            str(phash_dir),
            "--reports-dir",
            str(reports_dir),
            "--report-out",
            str(dry_report),
            "--migrate-to-moviehash",
            "--dry-run",
        ]
    )
    assert rc_dry == 0
    assert (ahash_dir / "Épisode [01].mkv.86.1fps.ahash").exists()

    live_report = reports_dir / "mig_live.jsonl"
    rc_live = bf.main(
        [
            "--ahash-dir",
            str(ahash_dir),
            "--phash-dir",
            str(phash_dir),
            "--reports-dir",
            str(reports_dir),
            "--report-out",
            str(live_report),
            "--migrate-to-moviehash",
            "--workers",
            "2",
        ]
    )
    assert rc_live == 0

    remaining = sorted(p.name for p in ahash_dir.iterdir())
    assert remaining == sorted(
        [
            f"{h_ph}.86.1fps.ahash",
            f"{h_ph}.json",
            f"{h_dup}.86.1fps.ahash",
            f"{h_dup}.json",
            f"{h_local}.86.1fps.ahash",
            f"{h_local}.json",
            f"{h_ftp}.86.1fps.ahash",
            f"{h_ftp}.json",
        ]
    )

    merged_dup = json.loads((ahash_dir / f"{h_dup}.json").read_text(encoding="utf-8"))
    assert merged_dup["moviehash"] == h_dup
    assert merged_dup["ahash_file"] == f"{h_dup}.86.1fps.ahash"
    assert merged_dup["selection"] == "verified_backfill"
    assert merged_dup["source_rel_path"] == "tvshow/Twin.Show.mkv"

    migrated_ph = json.loads((ahash_dir / f"{h_ph}.json").read_text(encoding="utf-8"))
    assert migrated_ph["moviehash"] == h_ph
    assert migrated_ph["ahash_file"] == f"{h_ph}.86.1fps.ahash"
    assert migrated_ph["media_name"] == "Épisode [01].mkv"
