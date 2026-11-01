"""Tests for scripts/o2switch_batch_summary.py (counts-only batch proof, printed in a public PR)."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
_spec = importlib.util.spec_from_file_location("o2switch_batch_summary", ROOT / "scripts" / "o2switch_batch_summary.py")
assert _spec and _spec.loader
summary = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(summary)

SECRET_NAME = "Some.Show.S01E01.very.long.name"
TOO_LONG_NEW = f'Cannot create output file "{SECRET_NAME}.phash.tmp": File name too long (os error 36)'
TOO_LONG_OLD = "Error: File name too long (os error 36)"


def _event(end_iso: str, success: bool, secs: float, error: str = "", resumed: int | None = None) -> str:
    rec: dict[str, object] = {
        "video_stem": SECRET_NAME,
        "end_iso": end_iso,
        "success": success,
        "wall_duration_secs": secs,
    }
    if error:
        rec["error_summary"] = error
    if resumed is not None:
        rec["resumed_frames"] = resumed
    return json.dumps(rec)


EVENTS = [
    _event("2026-10-20T10:00:00Z", False, 612.0, TOO_LONG_OLD),
    _event("2026-10-30T10:00:00Z", False, 1.2, TOO_LONG_NEW),
    _event("2026-10-30T10:01:00Z", False, 0.9, TOO_LONG_NEW),
    _event("2026-10-30T10:02:00Z", False, 30.0, "ffmpeg: connection reset"),
    _event("2026-10-30T11:00:00Z", True, 400.0, resumed=1500),
    _event("2026-10-30T12:00:00Z", True, 500.0, resumed=0),
    "not json {",
    "[1, 2]",
]


def test_summarize_counts_fast_fail_resume_and_ignores_garbage() -> None:
    s = summary.summarize_events(EVENTS)
    assert s == {
        "events": 6,
        "name_too_long": 3,
        "name_too_long_fast": 2,
        "name_too_long_fast_max_secs": 1.2,
        "completed": 2,
        "completed_resumed": 1,
        "resumed_frames": 1500,
    }


def test_summarize_since_keeps_only_newer_events() -> None:
    s = summary.summarize_events(EVENTS, since="2026-10-30")
    assert (s["events"], s["name_too_long"], s["name_too_long_fast"]) == (5, 2, 2)


def test_count_notes_counts_files_not_lines() -> None:
    logs = [
        "Extraction: no checkpoint, full decode\nExtraction: no checkpoint, full decode",
        "Extraction: resumed 1500 frames from checkpoint",
        "Extraction: checkpoint unreadable (bad magic), full decode",
        TOO_LONG_NEW,
        "nothing",
    ]
    assert summary.count_notes(logs) == {
        "name check before decode": 1,
        "no checkpoint, full decode": 1,
        "resumed from checkpoint": 1,
        "checkpoint unreadable": 1,
    }


def test_preflight_message_is_counted_from_full_logs_not_truncated_events() -> None:
    # Real case: error_summary keeps the last 300 chars, a > 255-byte name pushes the prefix out
    truncated = TOO_LONG_NEW.replace(SECRET_NAME, "x" * 300)[-300:]
    assert "Cannot create output file" not in truncated
    assert summary.count_notes([TOO_LONG_NEW.replace(SECRET_NAME, "x" * 300)])["name check before decode"] == 1


def test_main_prints_counts_only(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    (tmp_path / "data" / "reports").mkdir(parents=True)
    (tmp_path / "data" / "reports" / "batch_tvshow_nightly.jsonl").write_text("\n".join(EVENTS), encoding="utf-8")
    (tmp_path / "logs" / "workers").mkdir(parents=True)
    (tmp_path / "logs" / "workers" / f"{SECRET_NAME}.log").write_text(
        f"ftp://user:hunter2@nas.lan/{SECRET_NAME}.mkv\nExtraction: resumed 1500 frames from checkpoint",
        encoding="utf-8",
    )
    monkeypatch.chdir(tmp_path)
    assert summary.main(["2026-10-30"]) == 0
    out = capsys.readouterr().out
    assert "name too long        : 2 (os error 36); failed in <= 5 s: 2 (max 1.2 s)\n" in out
    assert "resumed from checkpoint   : 1" in out
    for secret in (SECRET_NAME, "Some.Show", "hunter2", "nas.lan", "ftp://"):
        assert secret not in out


def test_main_without_files_prints_zeroes(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.chdir(tmp_path)
    assert summary.main([]) == 0
    assert "events               : 0" in capsys.readouterr().out
