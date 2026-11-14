"""Unit and integration tests for batch_media_add_tvshow script."""

import json
import sys
import tempfile
from pathlib import Path

from scripts.ingestion.batch_media_add_tvshow import (
    _matches_stem_prefix,
    build_media_add_cmd,
    detect_alphabet_family,
    discover_stratified_tvshows,
    execute_media_add,
    load_targets_from_file,
    normalize_lang_code,
)


def test_normalize_lang_code():
    assert normalize_lang_code("fra") == "fr"
    assert normalize_lang_code("fre") == "fr"
    assert normalize_lang_code("eng") == "en"
    assert normalize_lang_code("spa") == "es"
    assert normalize_lang_code("ger") == "de"
    assert normalize_lang_code("deu") == "de"
    assert normalize_lang_code("und") == "und"
    assert normalize_lang_code("custom") == "custom"


def test_detect_alphabet_family():
    assert detect_alphabet_family("Hello world") == "Latin"
    assert detect_alphabet_family("Привет мир") == "Cyrillic"
    assert "CJK" in detect_alphabet_family("你好世界")


def test_load_targets_from_json_file():
    with tempfile.TemporaryDirectory() as tmp_dir:
        tmp_path = Path(tmp_dir)
        v1 = tmp_path / "show1" / "episode1.mkv"
        v2 = tmp_path / "show1" / "episode2.mkv"
        v3 = tmp_path / "show2" / "episode1.mkv"

        v1.parent.mkdir(parents=True)
        v3.parent.mkdir(parents=True)
        v1.write_bytes(b"dummy1")
        v2.write_bytes(b"dummy2")
        v3.write_bytes(b"dummy3")

        targets_json = tmp_path / "targets.json"
        targets_json.write_text(json.dumps([str(v1), str(v2), str(v3)]), encoding="utf-8")

        queue = load_targets_from_file(targets_json)
        assert len(queue) == 3
        paths = [item[0] for item in queue]
        assert v1 in paths
        assert v2 in paths
        assert v3 in paths


def test_load_targets_from_plaintext_file():
    with tempfile.TemporaryDirectory() as tmp_dir:
        tmp_path = Path(tmp_dir)
        v1 = tmp_path / "episode1.mkv"
        v2 = tmp_path / "episode2.mkv"
        v1.write_bytes(b"dummy1")
        v2.write_bytes(b"dummy2")

        targets_txt = tmp_path / "targets.txt"
        targets_txt.write_text(f"{v1}\n{v2}\n", encoding="utf-8")

        queue = load_targets_from_file(targets_txt)
        assert len(queue) == 2
        paths = [item[0] for item in queue]
        assert v1 in paths
        assert v2 in paths


def test_load_targets_missing_and_duplicates():
    with tempfile.TemporaryDirectory() as tmp_dir:
        tmp_path = Path(tmp_dir)
        v1 = tmp_path / "episode1.mkv"
        v1.write_bytes(b"dummy1")
        v_missing = tmp_path / "missing.mkv"

        targets_json = tmp_path / "targets.json"
        targets_json.write_text(json.dumps([str(v1), str(v1), str(v_missing)]), encoding="utf-8")

        queue = load_targets_from_file(targets_json)
        assert len(queue) == 1
        assert queue[0][0] == v1


def test_cli_targets_file_dry_run_execution():
    import subprocess

    with tempfile.TemporaryDirectory() as tmp_dir:
        tmp_path = Path(tmp_dir)
        v1 = tmp_path / "show1" / "test_ep1.mkv"
        v2 = tmp_path / "show1" / "test_ep2.mkv"
        v1.parent.mkdir(parents=True)
        v1.write_bytes(b"dummy1")
        v2.write_bytes(b"dummy2")

        targets_json = tmp_path / "selected.json"
        targets_json.write_text(json.dumps([str(v1), str(v2)]), encoding="utf-8")
        report_md = tmp_path / "report.md"

        # Run from tmp_path: the script writes data/reports/selected_batch_targets.json relative to cwd
        cmd = [
            sys.executable,
            str(Path(__file__).resolve().parents[2] / "scripts" / "ingestion" / "batch_media_add_tvshow.py"),
            "--targets-file",
            str(targets_json),
            "--report-path",
            str(report_md),
            "--dry-run",
            "--workers",
            "2",
        ]
        res = subprocess.run(cmd, capture_output=True, text=True, cwd=tmp_path)
        assert res.returncode == 0
        assert (tmp_path / "data" / "reports" / "selected_batch_targets.json").exists()
        assert "Discovered 2 candidate video(s) for direct processing." in res.stdout
        assert report_md.exists()


def test_discover_resumes_an_episode_whose_phash_is_named_by_moviehash(tmp_path: Path) -> None:
    show = tmp_path / "nas" / "Show"
    show.mkdir(parents=True)
    (show / "Episode.1.mkv").write_bytes(b"x")
    (show / "Episode.2.mkv").write_bytes(b"x")
    phash_dir = tmp_path / "phash"
    phash_dir.mkdir()
    (phash_dir / "e19d5212c9812cd6.25.0fps.phash").write_bytes(b"\x01" * 16)
    (phash_dir / "e19d5212c9812cd6.json").write_text(json.dumps({"media_name": "Episode.1.mkv"}), encoding="utf-8")

    fresh = discover_stratified_tvshows(tmp_path / "nas", phash_dir, tmp_path / "srt", fresh_only=True)

    # Episode.1 already has its .phash: only Episode.2 is virgin
    assert [path.name for path, _ in fresh] == ["Episode.2.mkv"]


def test_discover_ahash_requires_typed_sidecar_json(tmp_path: Path) -> None:
    """A .ahash without its typed `<stem>.json` sidecar stays pending in mode='ahash' and mode='both'."""
    show = tmp_path / "nas" / "Show"
    show.mkdir(parents=True)
    (show / "Episode.1.mkv").write_bytes(b"x")
    (show / "Episode.2.mkv").write_bytes(b"x")
    phash_dir = tmp_path / "phash"
    ahash_dir = tmp_path / "ahash"
    phash_dir.mkdir()
    ahash_dir.mkdir()

    # Episode.1 has moviehash .ahash + __PHASH__ sidecar, PLUS typed AhashSidecar in __AHASH__ -> done in ahash
    (phash_dir / "e19d5212c9812cd6.25.0fps.phash").write_bytes(b"\x01" * 16)
    (phash_dir / "e19d5212c9812cd6.json").write_text(json.dumps({"media_name": "Episode.1.mkv"}), encoding="utf-8")
    (ahash_dir / "e19d5212c9812cd6.86.1fps.ahash").write_bytes(b"\x02" * 16)
    (ahash_dir / "e19d5212c9812cd6.json").write_text(
        json.dumps({"media_name": "Episode.1.mkv", "selection": "single_track", "header": {"fps": 86.13}}),
        encoding="utf-8",
    )

    # Episode.2 has .phash and legacy .ahash WITHOUT __AHASH__/<hash>.json -> must still be selected
    (phash_dir / "a1b2c3d4e5f60718.25.0fps.phash").write_bytes(b"\x01" * 16)
    (phash_dir / "a1b2c3d4e5f60718.json").write_text(json.dumps({"media_name": "Episode.2.mkv"}), encoding="utf-8")
    (ahash_dir / "a1b2c3d4e5f60718.86.1fps.ahash").write_bytes(b"\x02" * 16)

    pending_ahash = discover_stratified_tvshows(
        tmp_path / "nas",
        phash_dir,
        tmp_path / "srt",
        fresh_only=False,
        mode="ahash",
        ahash_dir=ahash_dir,
    )
    assert [path.name for path, _ in pending_ahash] == ["Episode.2.mkv"]

    pending_both = discover_stratified_tvshows(
        tmp_path / "nas",
        phash_dir,
        tmp_path / "srt",
        fresh_only=False,
        mode="both",
        ahash_dir=ahash_dir,
    )
    assert [path.name for path, _ in pending_both] == ["Episode.2.mkv"]


def test_matches_stem_prefix_requires_dot_delimiter(tmp_path: Path) -> None:
    """Episode.10 must never mark Episode.1 as already processed."""
    assert not _matches_stem_prefix("Episode.1", ["Episode.10.mkv.25.0fps.phash", "Episode.11"])
    assert _matches_stem_prefix("Episode.1", ["Episode.1.mkv.25.0fps.phash"])
    assert _matches_stem_prefix("Episode.1", ["Episode.1"])

    show = tmp_path / "nas" / "Show"
    show.mkdir(parents=True)
    (show / "Episode.1.mkv").write_bytes(b"x")
    (show / "Episode.10.mkv").write_bytes(b"x")
    phash_dir = tmp_path / "phash"
    phash_dir.mkdir()
    (phash_dir / "Episode.10.mkv.25.0fps.phash").write_bytes(b"\x01" * 16)

    fresh = discover_stratified_tvshows(tmp_path / "nas", phash_dir, tmp_path / "srt", fresh_only=True)
    assert [path.name for path, _ in fresh] == ["Episode.1.mkv"]


def test_execute_media_add_passes_ahash_dir_and_skip_phash_by_mode(tmp_path: Path) -> None:
    fake_dbsrt = tmp_path / "fake_dbsrt.sh"
    argv_log = tmp_path / "argv.txt"
    fake_dbsrt.write_text(
        f'#!/bin/sh\nprintf "%s\\n" "$@" > "{argv_log}"\n'
        'printf \'{"certified_couplings": 0, "skipped_idempotent": 0}\\n\'\n',
        encoding="utf-8",
    )
    fake_dbsrt.chmod(0o755)

    video = tmp_path / "Show.2049.mkv"
    video.write_bytes(b"x")
    phash_dir = tmp_path / "phash"
    ahash_dir = tmp_path / "ahash"
    srt_dir = tmp_path / "srt"

    phash_cmd = build_media_add_cmd(fake_dbsrt, video, phash_dir, srt_dir, mode="phash", ahash_dir=ahash_dir)
    assert "--ahash-dir" not in phash_cmd
    assert "--skip-phash" not in phash_cmd

    both_cmd = build_media_add_cmd(fake_dbsrt, video, phash_dir, srt_dir, mode="both", ahash_dir=ahash_dir)
    assert both_cmd[both_cmd.index("--ahash-dir") + 1] == str(ahash_dir)
    assert "--skip-phash" not in both_cmd

    ok, cert, skip, err, *_ = execute_media_add(
        fake_dbsrt, video, phash_dir, srt_dir, mode="ahash", ahash_dir=ahash_dir
    )
    assert ok and cert == 0 and skip == 0 and err == ""
    recorded = argv_log.read_text(encoding="utf-8").splitlines()
    assert recorded[recorded.index("--ahash-dir") + 1] == str(ahash_dir)
    assert "--skip-phash" in recorded
