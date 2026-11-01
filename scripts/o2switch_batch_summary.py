#!/usr/bin/env python3
"""Counts-only summary of the o2switch batch worker, safe to publish (no media name, no path, no URL).

Runs on o2switch from the remote base dir, fed through ssh stdin (task o2switch:batch:summary):
the worker events file and logs/workers/*.log hold media names and may hold FTP credentials,
so only counters and durations ever leave this script. Python 3.6 there: no __future__ annotations.
"""

import json
import sys
from pathlib import Path
from typing import Dict, List

EVENTS_FILE = Path("data/reports/batch_tvshow_nightly.jsonl")
WORKER_LOGS_DIR = Path("logs/workers")
FAST_FAIL_SECS = 5.0
# Full worker logs, not error_summary: it keeps the last 300 chars, a > 255-byte name pushes the prefix out
NOTES = {
    "name check before decode": "Cannot create output file",
    "no checkpoint, full decode": "Extraction: no checkpoint, full decode",
    "resumed from checkpoint": "Extraction: resumed ",
    "checkpoint unreadable": "Extraction: checkpoint unreadable",
    "reused existing .phash": "Extraction: reused existing .phash",
}


def summarize_events(lines: List[str], since: str = "") -> Dict[str, object]:
    records = []
    for line in lines:
        try:
            rec = json.loads(line)
        except ValueError:
            continue
        if isinstance(rec, dict) and str(rec.get("end_iso", "")) >= since:
            records.append(rec)
    too_long = [r for r in records if "os error 36" in str(r.get("error_summary", ""))]
    fast = [float(r.get("wall_duration_secs", 0)) for r in too_long]
    fast = [d for d in fast if d <= FAST_FAIL_SECS]
    done = [r for r in records if r.get("success")]
    resumed = [int(r.get("resumed_frames", 0)) for r in done if int(r.get("resumed_frames", 0)) > 0]
    return {
        "events": len(records),
        "name_too_long": len(too_long),
        "name_too_long_fast": len(fast),
        "name_too_long_fast_max_secs": max(fast, default=0.0),
        "completed": len(done),
        "completed_resumed": len(resumed),
        "resumed_frames": sum(resumed),
    }


def count_notes(texts: List[str]) -> Dict[str, int]:
    return {label: sum(marker in text for text in texts) for label, marker in NOTES.items()}


def main(argv: List[str]) -> int:
    since = argv[0] if argv else ""
    lines = EVENTS_FILE.read_text(encoding="utf-8", errors="replace").splitlines() if EVENTS_FILE.is_file() else []
    s = summarize_events(lines, since)
    logs = [p.read_text(encoding="utf-8", errors="replace") for p in WORKER_LOGS_DIR.glob("*.log")]
    print(f"=== Batch worker summary (counts only, events since {since or 'the start'}) ===")
    print(f"events               : {s['events']}")
    print(
        f"name too long        : {s['name_too_long']} (os error 36); failed in <= {FAST_FAIL_SECS:.0f} s: "
        f"{s['name_too_long_fast']} (max {s['name_too_long_fast_max_secs']:.1f} s)"
    )
    print(
        f"completed            : {s['completed']}; resumed from checkpoint: {s['completed_resumed']} "
        f"({s['resumed_frames']} frames)"
    )
    print(f"worker logs          : {len(logs)}")
    for label, n in count_notes(logs).items():
        print(f"  {label:<26}: {n}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
