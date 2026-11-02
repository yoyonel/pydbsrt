"""Catalog self-recognition audit: title/episode matching rules and the counts-only report."""

import json
from pathlib import Path

import pytest

from scripts.o2switch_audit import report
from scripts.o2switch_audit_remote import cut_positions, logical_name, season_episode, title_matches, window_at


def test_logical_name_reads_the_media_name_of_a_moviehash_named_file(tmp_path: Path) -> None:
    (tmp_path / "0123456789abcdef.json").write_text(json.dumps({"media_name": "Big Buck Bunny"}))

    assert logical_name(str(tmp_path), "0123456789abcdef.25.000fps.phash") == "Big Buck Bunny.25.000fps.phash"
    assert logical_name(str(tmp_path), "fedcba9876543210.25.000fps.phash") == "fedcba9876543210.25.000fps.phash"
    assert logical_name(str(tmp_path), "Big.Buck.Bunny.25.000fps.phash") == "Big.Buck.Bunny.25.000fps.phash"


@pytest.mark.parametrize(
    ("match", "stem", "expected"),
    [
        ("Big Buck Bunny", "Big.Buck.Bunny.2008.1080p", True),
        ("Show Name - S01E02", "Show.Name.S01E02.720p", True),
        ("Bunny Big", "Big.Buck.Bunny.2008.1080p", False),
        ("Big Buck", "Big Bunny", False),
        ("", "Big Buck Bunny", False),
    ],
)
def test_title_matches_when_one_name_contains_the_other_in_order(match: str, stem: str, expected: bool) -> None:
    assert title_matches(match, stem) is expected


@pytest.mark.parametrize(
    ("stem", "expected"),
    [("Show.S01E02.720p", [1, 2]), ("show s3 e104", [3, 104]), ("Show_S10-E05", [10, 5]), ("Film 2008", None)],
)
def test_season_episode_is_parsed_from_the_file_name(stem: str, expected: list[int] | None) -> None:
    assert season_episode(stem) == expected


def test_window_at_takes_fps_times_seconds_frames_around_the_position() -> None:
    assert window_at(list(range(100)), 2.0, 5, 0.5) == list(range(45, 55))
    assert window_at(list(range(100)), 2.0, 5, 0.1) == list(range(5, 15))
    assert window_at(list(range(100)), 2.0, 5, 0.99) == list(range(90, 100))
    assert window_at(list(range(4)), 25.0, 10, 0.5) == [0, 1, 2, 3]


def test_cut_positions_spread_n_windows_and_one_cut_is_the_middle() -> None:
    assert cut_positions(1) == [0.5]
    assert cut_positions(5) == [0.1, 0.3, 0.5, 0.7, 0.9]


def row(
    stem: str, match: str, right: bool, stem_se: list[int] | None = None, resp_se: list | None = None, cut: float = 0.5
) -> dict:
    return {
        "stem": stem,
        "match": match,
        "http_ok": True,
        "right": right,
        "frames": 10,
        "distinct": 10,
        "stem_se": stem_se,
        "resp_se": resp_se or [None, None],
        "ms": 100,
        "cut": cut,
    }


def test_report_counts_titles_episodes_and_prints_no_media_name(tmp_path: Path, capsys: pytest.CaptureFixture) -> None:
    rows = [
        {"_meta": True, "eligible": 4, "catalog_media": 4},
        row("SecretFilmA", "SecretFilmA", True),
        row("SecretShow.S01E02", "SecretShow", True, [1, 2], [1, 2]),
        row("SecretShow.S01E03", "SecretShow", True, [1, 3], [1, 4]),
        row("SecretFilmB", "OtherFilm", False),
        {"stem": "SecretFilmC", "http_ok": False},
    ]
    out = tmp_path / "audit.jsonl"
    out.write_text("".join(json.dumps(r) + "\n" for r in rows))

    assert report(out) == 0

    text = capsys.readouterr().out
    assert "answered=4 no_reply=1" in text
    assert "title_right=3 (75.0%)" in text
    assert "episodes=2 title_right=2 episode_right=1 (50.0%)" in text
    assert "title right, other episode=1" in text
    assert "Secret" not in text and "OtherFilm" not in text


def test_report_counts_each_cut_of_a_file(tmp_path: Path, capsys: pytest.CaptureFixture) -> None:
    rows = [
        row("SecretFilmA", "SecretFilmA", True, cut=0.25),
        row("SecretFilmA", "OtherFilm", False, cut=0.75),
        row("SecretFilmB", "SecretFilmB", True, cut=0.25),
        row("SecretFilmB", "SecretFilmB", True, cut=0.75),
    ]
    out = tmp_path / "audit.jsonl"
    out.write_text("".join(json.dumps(r) + "\n" for r in rows))

    assert report(out) == 0

    text = capsys.readouterr().out
    assert "answered=4 no_reply=0" in text
    assert "cut=0.25 n=2 title_right=2 (100.0%)" in text
    assert "cut=0.75 n=2 title_right=1 (50.0%)" in text
    assert "files=2 any_cut_right=2 all_cuts_right=1 moviehash_without_sidecar=0" in text


def test_report_counts_moviehash_names_left_without_sidecar(tmp_path: Path, capsys: pytest.CaptureFixture) -> None:
    out = tmp_path / "audit.jsonl"
    out.write_text(json.dumps(row("0123456789abcdef", "SecretFilmA", False)) + "\n")

    assert report(out) == 0

    assert "moviehash_without_sidecar=1" in capsys.readouterr().out
