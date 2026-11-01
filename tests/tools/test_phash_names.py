import json
from pathlib import Path

from scripts.backfill_medias_pg import build_media_row
from scripts.phash_names import phash_logical_name, phash_media_name

MOVIEHASH = "0123456789abcdef"


def _write_phash(phash_dir: Path, name: str, frames: int = 3) -> Path:
    path = phash_dir / name
    path.write_bytes(b"\x00" * 8 * frames)
    return path


def test_legacy_phash_keeps_its_file_name(tmp_path: Path) -> None:
    phash = _write_phash(tmp_path, "Show.S01E01.mkv.25.0fps.phash")
    assert phash_logical_name(phash) == "Show.S01E01.mkv.25.0fps.phash"
    assert phash_media_name(phash) == "Show.S01E01.mkv.25.0fps"


def test_moviehash_phash_reads_its_media_name_from_the_sidecar(tmp_path: Path) -> None:
    phash = _write_phash(tmp_path, f"{MOVIEHASH}.25.0fps.phash")
    (tmp_path / f"{MOVIEHASH}.json").write_text(json.dumps({"media_name": "Show.S01E01.mkv"}))
    assert phash_logical_name(phash) == "Show.S01E01.mkv.25.0fps.phash"
    assert phash_media_name(phash) == "Show.S01E01.mkv.25.0fps"


def test_moviehash_phash_without_a_usable_sidecar_keeps_its_file_name(tmp_path: Path) -> None:
    phash = _write_phash(tmp_path, f"{MOVIEHASH}.25.0fps.phash")
    assert phash_logical_name(phash) == phash.name
    (tmp_path / f"{MOVIEHASH}.json").write_text("not json")
    assert phash_logical_name(phash) == phash.name
    (tmp_path / f"{MOVIEHASH}.json").write_text(json.dumps(["media_name"]))
    assert phash_logical_name(phash) == phash.name


def test_backfill_row_of_a_moviehash_phash_carries_the_media_name_and_fps(tmp_path: Path) -> None:
    phash = _write_phash(tmp_path, f"{MOVIEHASH}.24.0fps.phash", frames=48)
    (tmp_path / f"{MOVIEHASH}.json").write_text(json.dumps({"media_name": "Show.S01E02.mkv"}))
    row = build_media_row(phash, {})
    assert row is not None
    raw_name, fps, frame_count, phash_path = row[1], row[2], row[3], row[4]
    assert raw_name == "Show.S01E02.mkv"
    assert fps == 24.0
    assert frame_count == 48
    assert phash_path == phash.name
    assert row[8:10] == (1, 2)
