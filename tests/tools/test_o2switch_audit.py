"""Catalog self-recognition audit run detached on o2switch: matching rules, resume, lock, control and report."""

import json
import os
import signal
import struct
import threading
from pathlib import Path

import pytest

from scripts.infra.o2switch import o2switch_audit_remote as audit
from scripts.infra.o2switch.o2switch_audit_remote import (
    cut_positions,
    load,
    logical_name,
    report,
    run_file,
    season_episode,
    take_lock,
    title_matches,
    window_at,
)


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

    assert report(str(out)) == 0

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

    assert report(str(out)) == 0

    text = capsys.readouterr().out
    assert "answered=4 no_reply=0" in text
    assert "cut=0.25 n=2 title_right=2 (100.0%)" in text
    assert "cut=0.75 n=2 title_right=1 (50.0%)" in text
    assert "files=2 any_cut_right=2 all_cuts_right=1 moviehash_without_sidecar=0" in text


def test_report_counts_moviehash_names_left_without_sidecar(tmp_path: Path, capsys: pytest.CaptureFixture) -> None:
    out = tmp_path / "audit.jsonl"
    out.write_text(json.dumps(row("0123456789abcdef", "SecretFilmA", False)) + "\n")

    assert report(str(out)) == 0

    assert "moviehash_without_sidecar=1" in capsys.readouterr().out


def test_load_skips_a_line_cut_by_a_kill(tmp_path: Path) -> None:
    out = tmp_path / "audit.jsonl"
    out.write_text(
        json.dumps({"_meta": True, "eligible": 2}) + "\n" + json.dumps(row("A", "A", True)) + '\n{"stem": "B", "ma'
    )

    meta, rows = load(str(out))

    assert meta == {"_meta": True, "eligible": 2}
    assert [r["stem"] for r in rows] == ["A"]


@pytest.mark.parametrize("name", ["../etc/x", "a/b", ".hidden", "", "a b"])
def test_run_file_refuses_a_name_that_is_not_a_plain_file_name(name: str) -> None:
    with pytest.raises(SystemExit):
        run_file(name)


def test_run_file_keeps_the_run_under_the_audit_directory() -> None:
    assert run_file("recognize_2026-11-02") == os.path.join("audit", "recognize_2026-11-02.jsonl")


def test_take_lock_refuses_a_second_holder_until_the_first_one_is_gone(tmp_path: Path) -> None:
    lock = str(tmp_path / "audit.lock")
    fd = take_lock(lock)

    assert fd is not None
    assert take_lock(lock) is None
    os.close(fd)
    second = take_lock(lock)
    assert second is not None
    os.close(second)


def test_take_lock_waits_for_a_holder_that_is_exiting(tmp_path: Path) -> None:
    """Resume right after stop: the stopping audit still holds the lock for a few ms."""
    lock = str(tmp_path / "audit.lock")
    fd = take_lock(lock)
    assert fd is not None
    threading.Timer(0.2, os.close, [fd]).start()

    second = take_lock(lock, wait=2.0)

    assert second is not None
    os.close(second)


def test_take_lock_gives_up_after_the_wait(tmp_path: Path) -> None:
    lock = str(tmp_path / "audit.lock")
    fd = take_lock(lock)
    assert fd is not None

    assert take_lock(lock, wait=0.2) is None
    os.close(fd)


def held(path: str) -> int:
    fd = take_lock(path)
    assert fd is not None
    return fd


@pytest.fixture
def catalog(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> list[dict]:
    """Two ingested .phash files of 100 frames in a fake ~/dbsrt, and a daemon that answers every window."""
    monkeypatch.chdir(tmp_path)
    phash = tmp_path / "data" / "__PHASH__"
    phash.mkdir(parents=True)
    for name in ("FilmA.25.000fps.phash", "FilmB.25.000fps.phash"):
        (phash / name).write_bytes(b"".join(i.to_bytes(8, "big") for i in range(100)))
        os.utime(phash / name, (1_000, 1_000))
    (tmp_path / "data" / "catalog.dbphash").write_bytes(b"\0" * 12 + struct.pack("<I", 2))
    for d in ("audit", "run", "logs"):
        (tmp_path / d).mkdir()
    calls: list[dict] = []

    def fake_recognize(payload: dict, key: str) -> dict:
        calls.append(payload)
        return {"media_match": "FilmA", "found": True}

    monkeypatch.setattr(audit, "recognize", fake_recognize)
    return calls


def test_audit_run_writes_every_cut_of_every_ingested_file(catalog: list[dict]) -> None:
    assert audit.audit_run("r1", 1.0, 2, "key") == 0

    meta, rows = load(run_file("r1"))
    assert meta is not None and meta["eligible"] == 2
    assert sorted((r["file"], r["cut"]) for r in rows) == [
        ("FilmA.25.000fps.phash", 0.25),
        ("FilmA.25.000fps.phash", 0.75),
        ("FilmB.25.000fps.phash", 0.25),
        ("FilmB.25.000fps.phash", 0.75),
    ]
    assert len(catalog) == 4


def test_audit_run_resumes_without_asking_the_daemon_again_for_a_done_file(catalog: list[dict]) -> None:
    done = dict(row("FilmA", "FilmA", True), file="FilmA.25.000fps.phash")
    Path(run_file("r1")).write_text(json.dumps(done) + '\n{"stem": "cut by a ki')

    assert audit.audit_run("r1", 1.0, 2, "key") == 0

    _, rows = load(run_file("r1"))
    assert sorted({r["file"] for r in rows}) == ["FilmA.25.000fps.phash", "FilmB.25.000fps.phash"]
    assert len(catalog) == 2


def test_start_refuses_a_second_audit_while_one_holds_the_lock(
    catalog: list[dict], capsys: pytest.CaptureFixture, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(audit, "STOPPING_GRACE_SECS", 0.2)
    fd = held(audit.LOCK)

    assert audit.start("r2", 1.0, 2) == 1

    assert "already running" in capsys.readouterr().out
    assert not Path(run_file("r2")).exists()
    os.close(fd)


def test_status_reports_progress_and_whether_the_audit_runs(catalog: list[dict], capsys: pytest.CaptureFixture) -> None:
    audit.audit_run("r1", 1.0, 2, "key")
    Path(audit.STATE).write_text(json.dumps({"pid": 1, "run": "r1", "seconds": 1.0, "cuts": 2, "started": "t0"}))

    assert audit.status() == 0
    assert "not running" in capsys.readouterr().out

    fd = held(audit.LOCK)
    assert audit.status() == 0
    text = capsys.readouterr().out
    assert "RUNNING" in text and "run=r1" in text and "files=2/2" in text
    assert "Film" not in text
    os.close(fd)


def test_stop_never_signals_a_pid_that_is_not_the_audit(catalog: list[dict], capsys: pytest.CaptureFixture) -> None:
    fd = held(audit.LOCK)
    # The pid of pytest itself: a stale or reused pid must be refused, not killed.
    Path(audit.STATE).write_text(json.dumps({"pid": os.getpid(), "run": "r1"}))

    assert audit.stop() == 1

    assert "not the audit" in capsys.readouterr().out
    os.close(fd)


def test_stop_when_nothing_runs_is_a_no_op(catalog: list[dict], capsys: pytest.CaptureFixture) -> None:
    assert audit.stop() == 0
    assert "not running" in capsys.readouterr().out


def status_file() -> dict:
    return json.loads(Path(audit.STATUS).read_text())


def test_audit_run_publishes_a_counts_only_status_for_the_monitoring_page(catalog: list[dict]) -> None:
    assert audit.audit_run("r1", 1.0, 2, "key") == 0

    text = Path(audit.STATUS).read_text()
    assert "Film" not in text
    st = json.loads(text)
    assert (st["state"], st["run"], st["pid"]) == ("finished", "r1", os.getpid())
    assert (st["files_done"], st["eligible"], st["resumed_from"]) == (2, 2, 0)
    assert (st["answered"], st["title_right"], st["no_reply"]) == (4, 2, 0)
    assert (st["seconds"], st["cuts"]) == (1.0, 2)
    assert st["started"] and st["heartbeat"]
    assert not Path(audit.STATUS + ".tmp").exists()


def test_audit_status_says_how_many_files_were_done_before_this_process(catalog: list[dict]) -> None:
    done = dict(row("FilmA", "FilmA", True), file="FilmA.25.000fps.phash")
    Path(run_file("r1")).write_text(json.dumps(done) + "\n")

    audit.audit_run("r1", 1.0, 2, "key")

    assert (status_file()["resumed_from"], status_file()["files_done"]) == (1, 2)


def test_audit_status_is_refreshed_while_the_run_goes(catalog: list[dict], monkeypatch: pytest.MonkeyPatch) -> None:
    seen: list[tuple] = []

    def spy(payload: dict, key: str) -> dict:
        st = status_file()
        seen.append((st["state"], st["files_done"]))
        return {"media_match": "FilmA"}

    monkeypatch.setattr(audit, "PROGRESS_EVERY", 1)
    monkeypatch.setattr(audit, "recognize", spy)

    audit.audit_run("r1", 1.0, 2, "key")

    assert seen == [("running", 0), ("running", 0), ("running", 1), ("running", 1)]


def test_audit_status_is_failed_when_the_run_crashes(catalog: list[dict], monkeypatch: pytest.MonkeyPatch) -> None:
    def broken(payload: dict, key: str) -> dict:
        raise RuntimeError("socket gone")

    monkeypatch.setattr(audit, "recognize", broken)

    with pytest.raises(RuntimeError):
        audit.audit_run("r1", 1.0, 2, "key")

    assert status_file()["state"] == "failed"


def test_summarize_counts_episodes_and_no_reply_like_the_report() -> None:
    rows = [
        dict(row("Show S01E02", "Show", True, [1, 2], [1, 2]), file="a"),
        dict(row("Show S01E03", "Show", True, [1, 3], [1, 4]), file="b"),
        dict(row("Film", "", False), file="c", http_ok=False),
    ]

    assert audit.summarize(rows) == {
        "files_done": 3,
        "answered": 2,
        "no_reply": 1,
        "title_right": 2,
        "episodes": 2,
        "episode_right": 1,
    }


def test_status_prints_the_published_counts(catalog: list[dict], capsys: pytest.CaptureFixture) -> None:
    audit.audit_run("r1", 1.0, 2, "key")
    capsys.readouterr()

    assert audit.status() == 0

    out = capsys.readouterr().out
    assert "state=finished" in out and "files=2/2" in out and "title_right=2/4 (50.0%)" in out
    assert "Film" not in out


def sigterm_on_first_window(catalog: list[dict], monkeypatch: pytest.MonkeyPatch) -> None:
    """The daemon answers, then `audit stop` (or the portal) sends SIGTERM to the audit."""

    def stopped_here(payload: dict, key: str) -> dict:
        os.kill(os.getpid(), signal.SIGTERM)
        return {"media_match": "FilmA"}

    monkeypatch.setattr(audit, "recognize", stopped_here)


def test_a_sigterm_stops_the_run_cleanly_and_publishes_stopped(
    catalog: list[dict], monkeypatch: pytest.MonkeyPatch
) -> None:
    before = signal.getsignal(signal.SIGTERM)
    sigterm_on_first_window(catalog, monkeypatch)

    assert audit.audit_run("r1", 1.0, 2, "key") == 0

    st = status_file()
    assert (st["state"], st["run"], st["files_done"]) == ("stopped", "r1", 0)
    assert signal.getsignal(signal.SIGTERM) == before


def test_the_last_finished_run_stays_published_when_a_later_run_stops(
    catalog: list[dict], monkeypatch: pytest.MonkeyPatch
) -> None:
    assert audit.audit_run("r1", 1.0, 2, "key") == 0
    finished = status_file()
    assert json.loads(Path(audit.LAST_FINISHED).read_text()) == finished

    sigterm_on_first_window(catalog, monkeypatch)
    audit.audit_run("r2", 1.0, 2, "key")

    assert status_file()["state"] == "stopped"
    last = json.loads(Path(audit.LAST_FINISHED).read_text())
    assert (last["run"], last["state"], last["files_done"]) == ("r1", "finished", 2)
    assert "Film" not in Path(audit.LAST_FINISHED).read_text()


def test_a_sigterm_sent_before_the_run_is_ready_still_publishes_stopped(
    catalog: list[dict], monkeypatch: pytest.MonkeyPatch
) -> None:
    """Resume then stop at once: `start` blocks SIGTERM across the fork, so it waits for the handler."""
    signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGTERM})
    # pending, as when the stop beats the child's setup; thread-directed because pytest may run other
    # threads (the forked audit child has only one)
    signal.pthread_kill(threading.get_ident(), signal.SIGTERM)
    try:
        assert audit.audit_run("r1", 1.0, 2, "key") == 0
        st = status_file()
        assert (st["state"], st["files_done"]) == ("stopped", 0)
    finally:
        previous = signal.signal(signal.SIGTERM, signal.SIG_IGN)  # drain a SIGTERM still pending on RED
        signal.pthread_sigmask(signal.SIG_UNBLOCK, {signal.SIGTERM})
        signal.signal(signal.SIGTERM, previous)


def test_a_second_sigterm_while_stopping_does_not_break_the_stop(
    catalog: list[dict], monkeypatch: pytest.MonkeyPatch
) -> None:
    """Two stop clicks in a row: the second SIGTERM lands while `stopped` is being written."""
    sigterm_on_first_window(catalog, monkeypatch)
    write = audit.write_status

    def write_after_second_sigterm(status: dict, path: str = audit.STATUS) -> None:
        if status["state"] == "stopped":
            os.kill(os.getpid(), signal.SIGTERM)
        write(status, path)

    monkeypatch.setattr(audit, "write_status", write_after_second_sigterm)

    assert audit.audit_run("r1", 1.0, 2, "key") == 0
    assert status_file()["state"] == "stopped"


class FakeDaemon:
    """Portal routes of the daemon: an order takes `lag` telemetry reads to show, as a stopping process does."""

    def __init__(self, lag: int = 2, publish_on_double_stop: str = "stopped") -> None:
        self.state, self.target, self.lag = "stopped", "stopped", 0
        self.lag_reads, self.double_stop = lag, publish_on_double_stop
        self.now = 0.0

    def clock(self) -> float:
        return self.now

    def sleep(self, secs: float) -> None:
        self.now += secs

    def call(self, method: str, path: str) -> tuple[int, dict]:
        self.now += 0.05
        if path.endswith("/telemetry"):
            if self.lag > 0:
                self.lag -= 1
            else:
                self.state = self.target
            return 200, {"audit": {"state": self.state}}
        if path.endswith("/stop"):
            if self.target == "stopped":
                if self.state == "running":  # second stop while the first one is still landing
                    self.target = self.double_stop
                return 200, {}
            self.target, self.lag = "stopped", self.lag_reads
            return 200, {}
        if self.state == "running" or self.target == "running":
            return 409, {}
        self.target, self.lag = "running", self.lag_reads
        return 200, {}


def test_burst_follows_every_accepted_order_and_ends_on_the_last_one() -> None:
    d = FakeDaemon()

    rows, summary = audit.burst(3, d.call, d.clock, d.sleep)

    assert summary["anomalies"] == []
    assert summary["final_state"] == summary["expected"] == "stopped"
    accepted = [r for r in rows if r["code"] == 200 and r["waited"]]
    assert accepted and all(r["sync"] is not None for r in accepted)
    assert summary["sync_max"] >= summary["sync_p95"] > 0
    assert {r["order"] for r in rows} == {"stop", "resume"}


def test_burst_flags_a_double_stop_that_ends_interrupted() -> None:
    d = FakeDaemon(publish_on_double_stop="interrupted")

    _, summary = audit.burst(1, d.call, d.clock, d.sleep)

    assert any("interrupted" in a for a in summary["anomalies"])
