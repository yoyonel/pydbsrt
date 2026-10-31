#!/usr/bin/env python3
"""Per-minute proxies of o2switch LVE CPU faults, joinable with the cPanel "Resource Usage" table.

LVE faults are only visible in cPanel; CageFS hides /proc/lve and the cgroups. This records what
the account can see from inside, sampled every second over its own processes:

    watch      runs on o2switch (stdlib only), appends one JSON line per UTC minute to
               logs/faultwatch/YYYY-MM-DD.jsonl: CPU bursts, scheduler wait (throttle proxy),
               process/thread counts, spawns (ffmpeg = job churn, dbsrt-server = respawns).
    correlate  runs locally: joins those lines with a cPanel table pasted into a text file.

See docs/explanation/2026-10-31-o2switch-lve-cpu-faults-analysis.md.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any, Dict, List, Mapping, Sequence, Tuple

LVE_VCPUS = 12
CLK_TCK = os.sysconf("SC_CLK_TCK") if hasattr(os, "sysconf") else 100
KEEP_DAYS = 14
# cPanel shows CET; this is the offset that zeroed jobs during the 03:38-04:04 lull of 2026-10-30
DEFAULT_CPANEL_UTC_OFFSET_H = 1
WATCHED_SPAWNS = ("ffmpeg", "dbsrt-server")

# pid -> (comm, utime+stime ticks, threads, scheduler run_delay ns)
Sample = Dict[int, Tuple[str, int, int, int]]


def parse_stat(line: str) -> Tuple[str, int, int]:
    """Returns (comm, utime+stime ticks, num_threads) from a /proc/<pid>/stat line."""
    comm = line[line.index("(") + 1 : line.rindex(")")]
    rest = line[line.rindex(")") + 2 :].split()
    return comm, int(rest[11]) + int(rest[12]), int(rest[17])


def read_sample(proc: Path = Path("/proc"), uid: int | None = None) -> Sample:
    """Reads every process of `uid` (default: ours); processes exiting mid-read are skipped."""
    uid = os.getuid() if uid is None else uid
    sample: Sample = {}
    for entry in proc.iterdir():
        if not entry.name.isdigit():
            continue
        try:
            if entry.stat().st_uid != uid:
                continue
            comm, ticks, threads = parse_stat((entry / "stat").read_text())
        except (OSError, ValueError, IndexError):
            continue
        # /proc/<pid>/schedstat only covers the main thread: decoders wait in their worker threads
        run_delay = 0
        for task in (entry / "task").glob("*/schedstat"):
            try:
                run_delay += int(task.read_text().split()[1])
            except (OSError, ValueError, IndexError):
                continue
        sample[int(entry.name)] = (comm, ticks, threads, run_delay)
    return sample


def second_metrics(prev: Sample, cur: Sample, dt: float) -> Dict[str, float]:
    """CPU % of the LVE limit and scheduler wait over `dt`; new pids count from zero.

    shortcut: CPU burned by a pid during the second it exits is lost, fine for burst detection.
    """
    ticks = run_delay = 0
    for pid, (_, t, _, rd) in cur.items():
        old = prev.get(pid)
        ticks += t - old[1] if old else t
        # a thread exiting removes its share from the sum: clamp instead of going negative
        run_delay += max(rd - old[3], 0) if old else rd
    new = [cur[pid][0] for pid in cur.keys() - prev.keys()]
    out = {
        "cpu_pct": ticks / CLK_TCK / dt / LVE_VCPUS * 100,
        "run_delay_ms": run_delay / 1e6 / dt,
        "nproc": len(cur),
        "threads": sum(v[2] for v in cur.values()),
        "spawns": len(new),
        "ffmpeg": sum(1 for v in cur.values() if v[0] == "ffmpeg"),
    }
    for name in WATCHED_SPAWNS:
        out[f"{name}_spawns"] = sum(1 for c in new if c == name)
    return out


def minute_record(minute: datetime, seconds: List[Dict[str, float]]) -> Dict[str, object]:
    """Aggregates one minute of per-second metrics; keys are what `correlate` buckets on."""
    cpu = sorted(s["cpu_pct"] for s in seconds)
    rd = [s["run_delay_ms"] for s in seconds]
    rec: Dict[str, object] = {
        "minute": minute.strftime("%Y-%m-%dT%H:%MZ"),
        "n": len(seconds),
        "cpu_mean": round(sum(cpu) / len(cpu), 1),
        "cpu_p90": round(cpu[int(0.9 * (len(cpu) - 1))], 1),
        "cpu_max": round(cpu[-1], 1),
        "secs_over_90": sum(1 for c in cpu if c >= 90),
        "secs_over_100": sum(1 for c in cpu if c >= 100),
        "run_delay_ms_mean": round(sum(rd) / len(rd), 1),
        "run_delay_ms_max": round(max(rd), 1),
        "nproc_max": max(s["nproc"] for s in seconds),
        "threads_max": max(s["threads"] for s in seconds),
        "ffmpeg_max": max(s["ffmpeg"] for s in seconds),
        "spawns": sum(s["spawns"] for s in seconds),
    }
    for name in WATCHED_SPAWNS:
        rec[f"{name}_spawns"] = sum(s[f"{name}_spawns"] for s in seconds)
    return rec


def watch(out_dir: Path) -> None:
    out_dir.mkdir(parents=True, exist_ok=True)
    prev, prev_t = read_sample(), time.monotonic()
    minute = datetime.now(UTC).replace(second=0, microsecond=0)
    seconds: List[Dict[str, float]] = []
    while True:
        time.sleep(1 - time.time() % 1)
        cur, now_t = read_sample(), time.monotonic()
        now = datetime.now(UTC).replace(second=0, microsecond=0)
        if now != minute and seconds:
            with open(out_dir / f"{minute:%Y-%m-%d}.jsonl", "a") as fh:
                fh.write(json.dumps(minute_record(minute, seconds)) + "\n")
            if now.date() != minute.date():
                prune(out_dir, now)
            seconds = []
        minute = now
        seconds.append(second_metrics(prev, cur, now_t - prev_t))
        prev, prev_t = cur, now_t


def prune(out_dir: Path, now: datetime) -> None:
    cutoff = f"{now - timedelta(days=KEEP_DAYS):%Y-%m-%d}.jsonl"
    for f in out_dir.glob("*.jsonl"):
        if f.name < cutoff:
            f.unlink()


_ROW_TS = re.compile(r"^\d\d-\d\d \d\d:\d\d$")
_UNITS = (("KB/s", 1 / 1024), ("MB/s", 1.0), ("G", 1024.0), ("M", 1.0), ("K", 1 / 1024))
# SPEED, PMEM, IO, EP, NPROC, IOPS, each as (average, limit, faults)
_CPANEL_COLS = ("speed", "pmem", "io", "ep", "nproc", "iops")


def _cpanel_number(raw: str) -> float:
    raw = raw.rstrip("%")
    for suffix, factor in _UNITS:
        if raw.endswith(suffix):
            return float(raw[: -len(suffix)]) * factor
    return float(raw)


def parse_cpanel_table(text: str, year: int) -> List[Dict[str, Any]]:
    """Parses the cPanel Resource Usage table copied as one cell per line (From, To, 18 values).

    Values: speed %, pmem MiB, io MiB/s, ep, nproc, iops; `<col>_faults` per column. Rows with a
    wrong value count are skipped. `local` is the cPanel wall-clock minute (no timezone).
    """
    lines = [line.strip() for line in text.splitlines() if line.strip()]
    rows = []
    i = 0
    while i + 1 < len(lines):
        values = lines[i + 2 : i + 20]
        if not (_ROW_TS.match(lines[i]) and _ROW_TS.match(lines[i + 1])):
            i += 1
            continue
        if len(values) != 18 or any(_ROW_TS.match(v) for v in values):
            i += 2
            continue
        row: Dict[str, Any] = {"local": datetime.strptime(f"{year}-{lines[i]}", "%Y-%d-%m %H:%M")}
        for k, col in enumerate(_CPANEL_COLS):
            row[col] = _cpanel_number(values[3 * k])
            row[f"{col}_faults"] = int(values[3 * k + 2])
        rows.append(row)
        i += 20
    return rows


def join(cpanel: Sequence[Mapping[str, Any]], watch_records: Sequence[Mapping[str, Any]], utc_offset_h: int):
    """Pairs each cPanel minute with the watcher minute at the same instant."""
    by_minute = {r["minute"]: r for r in watch_records}
    pairs = []
    for row in cpanel:
        key = (row["local"] - timedelta(hours=utc_offset_h)).strftime("%Y-%m-%dT%H:%MZ")
        if key in by_minute:
            pairs.append((row, by_minute[key]))
    return pairs


def _pearson(xs: List[float], ys: List[float]) -> float:
    mx, my = sum(xs) / len(xs), sum(ys) / len(ys)
    cov = sum((x - mx) * (y - my) for x, y in zip(xs, ys))
    var = (sum((x - mx) ** 2 for x in xs) * sum((y - my) ** 2 for y in ys)) ** 0.5
    return cov / var if var else float("nan")


def _feature_line(key: str, xs: List[float], faults: List[float]) -> str:
    """Pearson r with faults/min, then P(fault) per quartile of the feature."""
    ranked = sorted(zip(xs, faults))
    quarters = [ranked[q * len(ranked) // 4 : (q + 1) * len(ranked) // 4] for q in range(4)]
    probs = " ".join(f"<={q[-1][0]:g}:{sum(1 for _, f in q if f) / len(q):.0%}" for q in quarters if q)
    return f"  {key:<22} r={_pearson(xs, faults):5.2f}  {probs}"


def correlate(cpanel_file: Path, watch_dir: Path, year: int, utc_offset_h: int) -> None:
    cpanel = parse_cpanel_table(cpanel_file.read_text(encoding="utf-8"), year)
    records = [json.loads(line) for f in sorted(watch_dir.glob("*.jsonl")) for line in f.open()]
    pairs = join(cpanel, records, utc_offset_h)
    print(f"cPanel minutes {len(cpanel)}, watcher minutes {len(records)}, joined {len(pairs)}")
    if len(pairs) < 2:
        return
    faults = [float(c["speed_faults"]) for c, _ in pairs]
    print(f"fault minutes {sum(1 for f in faults if f)}, faults {int(sum(faults))}")
    print(
        "corr(cPanel speed, watcher cpu_mean) =",
        round(_pearson([c["speed"] for c, _ in pairs], [w["cpu_mean"] for _, w in pairs]), 2),
    )
    features = [k for k, v in pairs[0][1].items() if isinstance(v, int | float) and k != "n"]
    print("\ncorrelation with faults/min, then P(fault) by feature quartile")
    for key in features:
        print(_feature_line(key, [float(w[key]) for _, w in pairs], faults))


def main(argv: List[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").split("\n")[0])
    sub = parser.add_subparsers(dest="cmd", required=True)
    w = sub.add_parser("watch")
    w.add_argument("--out-dir", type=Path, default=Path("logs/faultwatch"))
    c = sub.add_parser("correlate")
    c.add_argument("cpanel_file", type=Path, help="cPanel Resource Usage table pasted as text")
    c.add_argument("--watch-dir", type=Path, default=Path("data/reports/faultwatch"))
    c.add_argument("--year", type=int, default=datetime.now().year)
    c.add_argument("--utc-offset", type=int, default=DEFAULT_CPANEL_UTC_OFFSET_H)
    args = parser.parse_args(argv)
    if args.cmd == "watch":
        watch(args.out_dir)
    else:
        correlate(args.cpanel_file, args.watch_dir, args.year, args.utc_offset)
    return 0


if __name__ == "__main__":
    sys.exit(main())
