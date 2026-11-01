import json
from pathlib import Path

from scripts.phash_dedupe import dedupe_legacy_twins

MOVIEHASH = "0123456789abcdef"
OTHER_MOVIEHASH = "fedcba9876543210"


def _phash(phash_dir: Path, name: str, payload: bytes) -> Path:
    path = phash_dir / name
    path.write_bytes(payload)
    return path


def _sidecar(phash_dir: Path, moviehash: str, media_name: str) -> None:
    (phash_dir / f"{moviehash}.json").write_text(json.dumps({"media_name": media_name}))


def test_dry_run_counts_an_identical_legacy_twin_and_keeps_it(tmp_path: Path) -> None:
    _phash(tmp_path, f"{MOVIEHASH}.25.0fps.phash", b"\x01" * 16)
    _sidecar(tmp_path, MOVIEHASH, "Show.S01E01.mkv")
    legacy = _phash(tmp_path, "Show.S01E01.mkv.25.0fps.phash", b"\x01" * 16)

    counts = dedupe_legacy_twins(tmp_path, apply=False)

    assert counts == {"moviehash": 1, "legacy_twin": 1, "identical": 1, "differs": 0, "removed": 0}
    assert legacy.exists()


def test_apply_removes_only_byte_identical_legacy_twins(tmp_path: Path) -> None:
    moviehash_file = _phash(tmp_path, f"{MOVIEHASH}.25.0fps.phash", b"\x01" * 16)
    _sidecar(tmp_path, MOVIEHASH, "Show.S01E01.mkv")
    twin = _phash(tmp_path, "Show.S01E01.mkv.25.0fps.phash", b"\x01" * 16)
    _phash(tmp_path, f"{OTHER_MOVIEHASH}.25.0fps.phash", b"\x02" * 16)
    _sidecar(tmp_path, OTHER_MOVIEHASH, "Show.S01E02.mkv")
    differing = _phash(tmp_path, "Show.S01E02.mkv.25.0fps.phash", b"\x03" * 16)
    unrelated = _phash(tmp_path, "Show.S01E03.mkv.25.0fps.phash", b"\x01" * 16)

    counts = dedupe_legacy_twins(tmp_path, apply=True)

    assert counts == {"moviehash": 2, "legacy_twin": 2, "identical": 1, "differs": 1, "removed": 1}
    assert not twin.exists()
    assert moviehash_file.exists()
    assert differing.exists()
    assert unrelated.exists()


def test_moviehash_phash_without_sidecar_never_matches_a_legacy_file(tmp_path: Path) -> None:
    _phash(tmp_path, f"{MOVIEHASH}.25.0fps.phash", b"\x01" * 16)
    legacy = _phash(tmp_path, f"{MOVIEHASH}.25.0fps.phash.bak", b"\x01" * 16)

    counts = dedupe_legacy_twins(tmp_path, apply=True)

    assert counts == {"moviehash": 0, "legacy_twin": 0, "identical": 0, "differs": 0, "removed": 0}
    assert legacy.exists()
