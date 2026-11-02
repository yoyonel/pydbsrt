"""Catalog self-recognition audit of the o2switch engine (task o2switch:audit / o2switch:audit:report).

`run OUT`: pipes scripts/o2switch_audit_remote.py to o2switch over SSH, chunk by chunk, and appends its
rows to OUT. Resumes from the rows already in OUT, so an interrupted run continues where it stopped.
`report OUT`: prints counts only. OUT holds media names: keep it under the gitignored target/ tree.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

REMOTE_SCRIPT = Path(__file__).with_name("o2switch_audit_remote.py")
# The bare python3 on o2switch is the system 3.6; bin/python3 is linked to /opt/alt/python313 by o2switch:sync.
REMOTE_PYTHON = "./bin/python3"
DISTINCT_BINS = (0.0, 0.05, 0.10, 0.25, 0.50, 1.01)
MOVIEHASH = re.compile(r"[0-9a-fA-F]{16}")


def load(out: Path) -> tuple[dict | None, list[dict]]:
    meta, rows = None, []
    if out.exists():
        for line in out.read_text().splitlines():
            row = json.loads(line)
            if row.get("_meta"):
                meta = row
            else:
                rows.append(row)
    return meta, rows


def progress(out: Path) -> tuple[int, int | None]:
    """Files already audited (one row per cut) and the eligible count once known."""
    meta, rows = load(out)
    return len({r.get("file", r["stem"]) for r in rows}), meta["eligible"] if meta else None


def run(out: Path, chunk: int, seconds: float, cuts: int) -> int:
    out.parent.mkdir(parents=True, exist_ok=True)
    done, eligible = progress(out)
    remote = REMOTE_SCRIPT.read_text()
    while eligible is None or done < eligible:
        r = subprocess.run(
            ["ssh", "o2switch", f"cd dbsrt && {REMOTE_PYTHON} -I - {done} {chunk} {seconds} {cuts}"],
            input=remote,
            capture_output=True,
            text=True,
            timeout=3600,
        )
        new = [line for line in r.stdout.splitlines() if line.startswith("{")]
        with out.open("a") as fh:
            fh.writelines(line + "\n" for line in new)
        done, eligible = progress(out)
        print(f"rc={r.returncode} rows={len(new)} done={done}/{eligible}", flush=True)
        if r.returncode != 0 or not new:
            return 1
    return 0


def pct(n: int, d: int) -> str:
    return f"{100 * n / d:.1f}%" if d else "n/a"


def row_key(r: dict) -> tuple:
    return r.get("file", r["stem"]), r.get("cut")


def report(out: Path) -> int:
    meta, all_rows = load(out)
    rows = list({row_key(r): r for r in all_rows if r["http_ok"]}.values())
    n, right = len(rows), sum(r["right"] for r in rows)
    no_reply = len({row_key(r) for r in all_rows}) - n
    print(f"eligible={meta['eligible'] if meta else '?'} answered={n} no_reply={no_reply}")
    print(f"title_right={right} ({pct(right, n)}) empty_answer={sum(not r['match'] for r in rows)}")
    eps = [r for r in rows if r["stem_se"]]
    eps_title = [r for r in eps if r["right"]]
    exact = sum(r["resp_se"] == r["stem_se"] for r in eps_title)
    no_se = sum(r["resp_se"] == [None, None] for r in eps_title)
    print(f"episodes={len(eps)} title_right={len(eps_title)} episode_right={exact} ({pct(exact, len(eps))})")
    print(f"  title right, other episode={len(eps_title) - exact - no_se} title right, no season/episode={no_se}")
    movies = [r for r in rows if not r["stem_se"]]
    movies_right = sum(r["right"] for r in movies)
    print(f"movies={len(movies)} title_right={movies_right} ({pct(movies_right, len(movies))})")
    report_distinct_bins(rows)
    report_cuts(rows)
    ms = [r["ms"] for r in rows if "ms" in r]
    if ms:
        print(f"request_ms mean={sum(ms) / len(ms):.0f} max={max(ms)} total_min={sum(ms) / 60000:.1f}")
    return 0


def report_cuts(rows: list[dict]) -> None:
    for cut in sorted({r.get("cut") or 0.5 for r in rows}):
        b = [r for r in rows if (r.get("cut") or 0.5) == cut]
        right = sum(r["right"] for r in b)
        print(f"cut={cut:.2f} n={len(b)} title_right={right} ({pct(right, len(b))})")
    by_file: dict[str, list[bool]] = {}
    for r in rows:
        by_file.setdefault(r.get("file", r["stem"]), []).append(r["right"])
    print(
        f"files={len(by_file)} any_cut_right={sum(any(v) for v in by_file.values())}"
        f" all_cuts_right={sum(all(v) for v in by_file.values())}"
        f" moviehash_without_sidecar={len({r['stem'] for r in rows if MOVIEHASH.fullmatch(r['stem'])})}"
    )


def report_distinct_bins(rows: list[dict]) -> None:
    print("distinct/frames bin: n wrong%")
    for lo, hi in zip(DISTINCT_BINS, DISTINCT_BINS[1:]):
        b = [r for r in rows if lo <= r["distinct"] / max(r["frames"], 1) < hi]
        print(f"  [{lo:.2f},{hi:.2f}) n={len(b):5d} wrong={pct(sum(not r['right'] for r in b), len(b))}")


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("run")
    r.add_argument("out", type=Path)
    r.add_argument("--chunk", type=int, default=200)
    r.add_argument("--seconds", type=float, default=10.0)
    r.add_argument("--cuts", type=int, default=1)
    sub.add_parser("report").add_argument("out", type=Path)
    a = p.parse_args()
    return run(a.out, a.chunk, a.seconds, a.cuts) if a.cmd == "run" else report(a.out)


if __name__ == "__main__":
    sys.exit(main())
