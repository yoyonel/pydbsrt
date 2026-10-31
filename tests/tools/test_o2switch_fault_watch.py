"""Hermetic unit tests for scripts/o2switch_fault_watch.py."""

from __future__ import annotations

from datetime import UTC, datetime

import pytest

from scripts.o2switch_fault_watch import (
    CLK_TCK,
    LVE_VCPUS,
    join,
    minute_record,
    parse_cpanel_table,
    parse_stat,
    read_sample,
    second_metrics,
)


@pytest.mark.parametrize(
    ("line", "expected"),
    [
        ("42 (ffmpeg) S 1 42 42 0 0 0 0 0 0 0 150 50 0 0 20 0 12 0 1\n", ("ffmpeg", 200, 12)),
        ("43 (ff (a) b) R 42 42 42 0 0 0 0 0 0 0 10 0 0 0 20 0 1 0 1\n", ("ff (a) b", 10, 1)),
    ],
)
def test_parse_stat_handles_comm_with_spaces_and_parens(line, expected):
    assert parse_stat(line) == expected


def test_read_sample_skips_other_uids_and_vanished_pids(tmp_path):
    ok = tmp_path / "10"
    ok.mkdir()
    (ok / "stat").write_text("10 (dbsrt) S 1 1 1 0 0 0 0 0 0 0 5 5 0 0 20 0 4 0 1\n")
    (ok / "schedstat").write_text("1000 999 3\n")  # main thread only: must not be used
    for tid, line in (("10", "1000 2500000 3\n"), ("12", "500 500000 1\n"), ("13", "garbage\n")):
        (ok / "task" / tid).mkdir(parents=True)
        (ok / "task" / tid / "schedstat").write_text(line)
    (tmp_path / "11").mkdir()  # exited between listdir and read: no stat file
    (tmp_path / "self").mkdir()
    uid = ok.stat().st_uid
    assert read_sample(tmp_path, uid) == {10: ("dbsrt", 10, 4, 3000000)}
    assert read_sample(tmp_path, uid + 1) == {}


def test_second_metrics_uses_deltas_and_counts_spawns():
    prev = {1: ("dbsrt", 1000, 4, 0), 2: ("ffmpeg", 500, 12, 1_000_000)}
    # pid 2 used 6 cores for 1 s, pid 1 idle, pid 3 is a new ffmpeg with 6 cores of CPU so far
    cur = {
        1: ("dbsrt", 1000, 4, 0),
        2: ("ffmpeg", 500 + 6 * CLK_TCK, 12, 3_000_000),
        3: ("ffmpeg", 6 * CLK_TCK, 12, 0),
    }
    m = second_metrics(prev, cur, 1.0)
    assert m["cpu_pct"] == pytest.approx(12 / LVE_VCPUS * 100)
    assert m["run_delay_ms"] == pytest.approx(2.0)
    assert (m["nproc"], m["threads"], m["spawns"], m["ffmpeg"]) == (3, 28, 1, 2)
    assert (m["ffmpeg_spawns"], m["dbsrt-server_spawns"]) == (1, 0)
    # a thread exit shrinks the summed run_delay: never a negative wait
    shrunk = {pid: (c, t, n, 0) for pid, (c, t, n, _) in prev.items()}
    assert second_metrics(prev, shrunk, 1.0)["run_delay_ms"] == 0


def _second(cpu, spawns=0, server=0):
    return {
        "cpu_pct": cpu,
        "run_delay_ms": cpu / 10,
        "nproc": 90,
        "threads": 300,
        "spawns": spawns,
        "ffmpeg": 4,
        "ffmpeg_spawns": spawns,
        "dbsrt-server_spawns": server,
    }


def test_minute_record_counts_bursts_over_the_limit():
    seconds = [_second(80)] * 57 + [_second(95), _second(101, spawns=2), _second(120, server=1)]
    rec = minute_record(datetime(2026, 10, 31, 0, 53, tzinfo=UTC), seconds)
    assert rec["minute"] == "2026-10-31T00:53Z"
    assert (rec["n"], rec["secs_over_90"], rec["secs_over_100"]) == (60, 3, 2)
    assert (rec["cpu_max"], rec["cpu_p90"]) == (120, 80)
    assert (rec["spawns"], rec["ffmpeg_spawns"], rec["dbsrt-server_spawns"]) == (2, 2, 1)


_CPANEL = "\n".join(
    ["From", "To", "SPEED", "A", "L", "F"]
    + ["30-10 00:00", "30-10 00:01", "1013%", "1200%", "2", "517.93M", "48G", "0", "1.75MB/s", "42MB/s", "0"]
    + ["0", "0", "0", "88", "400", "0", "0", "2048", "0"]
    # truncated row (copy glitch): skipped, next row still parsed
    + ["30-10 00:01", "30-10 00:02", "815%", "1200%"]
    + ["30-10 00:02", "30-10 00:03", "785%", "1200%", "0", "1.5G", "48G", "0", "512KB/s", "42MB/s", "0"]
    + ["1", "0", "0", "95", "400", "0", "0", "2048", "0"]
)


def test_parse_cpanel_table_units_and_glitches():
    rows = parse_cpanel_table(_CPANEL, 2026)
    assert [r["local"] for r in rows] == [datetime(2026, 10, 30, 0, 0), datetime(2026, 10, 30, 0, 2)]
    first, last = rows
    assert (first["speed"], first["speed_faults"], first["nproc"]) == (1013, 2, 88)
    assert first["pmem"] == pytest.approx(517.93)
    assert first["io"] == pytest.approx(1.75)
    assert last["pmem"] == pytest.approx(1536)
    assert last["io"] == pytest.approx(0.5)
    assert (last["ep"], last["nproc"]) == (1, 95)


@pytest.mark.parametrize(("offset", "joined"), [(1, ["2026-10-29T23:00Z"]), (0, ["2026-10-30T00:00Z"])])
def test_join_converts_cpanel_local_time_to_utc(offset, joined):
    cpanel = parse_cpanel_table(_CPANEL, 2026)[:1]
    records = [{"minute": "2026-10-29T23:00Z"}, {"minute": "2026-10-30T00:00Z"}]
    assert [w["minute"] for _, w in join(cpanel, records, offset)] == joined
