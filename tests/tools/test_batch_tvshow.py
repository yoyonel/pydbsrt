"""Unit and integration tests for batch_media_add_tvshow script."""

import json
import tempfile
from pathlib import Path

from scripts.batch_media_add_tvshow import (
    detect_alphabet_family,
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

        cmd = [
            "uv",
            "run",
            "python",
            "scripts/batch_media_add_tvshow.py",
            "--targets-file",
            str(targets_json),
            "--report-path",
            str(report_md),
            "--dry-run",
            "--workers",
            "2",
        ]
        res = subprocess.run(cmd, capture_output=True, text=True)
        assert res.returncode == 0
        assert "Discovered 2 candidate video(s) for direct processing." in res.stdout
        assert report_md.exists()
