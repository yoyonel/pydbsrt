#!/usr/bin/env python3
"""Generic, zero-LLM profiler and bottleneck detector for Antigravity Agent Skill transcripts."""

from __future__ import annotations

import argparse
import json
import re
import sys
from datetime import datetime
from pathlib import Path
from typing import Any

CREATED_RE = re.compile(r"Created At:\s*(\S+)")
COMPLETED_RE = re.compile(r"Completed At:\s*(\S+)")

ROUND_DECIMALS: int = 2
THINKING_LOOP_THRESHOLD_CHARS: int = 10000
PARALLEL_STAMPEDE_MIN_CALLS: int = 3
MAX_CMD_PREVIEW_CHARS: int = 120


def _parse_iso(ts: str) -> datetime:
    """Parses an ISO-8601 timestamp into a timezone-aware datetime."""
    return datetime.fromisoformat(ts.replace("Z", "+00:00"))


def _measure_tool_window_seconds(steps: list[dict[str, Any]]) -> float:
    """Computes union of [Created At, Completed At] intervals across all tool outputs."""
    intervals: list[tuple[float, float]] = []
    for step in steps:
        content = str(step.get("content") or "")
        m_start = CREATED_RE.search(content)
        m_end = COMPLETED_RE.search(content)
        if m_start and m_end:
            s_ep = _parse_iso(m_start.group(1)).timestamp()
            e_ep = _parse_iso(m_end.group(1)).timestamp()
            if e_ep >= s_ep:
                intervals.append((s_ep, e_ep))
    if not intervals:
        return 0.0
    intervals.sort()
    merged: list[list[float]] = [[intervals[0][0], intervals[0][1]]]
    for start, end in intervals[1:]:
        if start <= merged[-1][1]:
            merged[-1][1] = max(merged[-1][1], end)
        else:
            merged.append([start, end])
    return round(sum(end - start for start, end in merged), ROUND_DECIMALS)


def _summarize_tool_call(tc: dict[str, Any]) -> str:
    """Builds a compact human-readable label for a single tool call."""
    name = str(tc.get("name") or tc.get("tool_name") or "unknown")
    args = tc.get("arguments") or tc.get("args") or {}
    if isinstance(args, str):
        try:
            args = json.loads(args)
        except ValueError:
            args = {}
    if not isinstance(args, dict):
        return name
    if "Path" in args:
        return f"{name}[{Path(str(args['Path'])).name}]"
    if "CommandLine" in args:
        cmd_preview = str(args["CommandLine"])[:MAX_CMD_PREVIEW_CHARS]
        return f"{name}[{cmd_preview}]"
    return name


def _detect_step_anomalies(step: dict[str, Any]) -> list[str]:
    """Flags empirically known skill performance traps in a single step."""
    flags: list[str] = []
    thinking = str(step.get("thinking") or "")
    if len(thinking) >= THINKING_LOOP_THRESHOLD_CHARS:
        flags.append(f"THINKING_LOOP_SPIKE:{len(thinking)}c")

    content = str(step.get("content") or "")
    if "rtk/tee/" in content or "[full output:" in content:
        flags.append("RTK_TEE_TRUNCATION")

    tcs = step.get("tool_calls") or []
    run_cmd_count = sum(1 for tc in tcs if (tc.get("name") or tc.get("tool_name")) == "run_command")
    if run_cmd_count >= PARALLEL_STAMPEDE_MIN_CALLS:
        flags.append(f"PARALLEL_TOOL_STAMPEDE:{run_cmd_count}_calls")

    for tc in tcs:
        label = _summarize_tool_call(tc)
        if label == "read_file[output.txt]":
            flags.append("OUTPUT_TXT_SPILL_10KB")
    return flags


def _find_first_timestamp(steps: list[dict[str, Any]]) -> datetime | None:
    """Returns the earliest ISO timestamp present in steps, or None."""
    timestamps = [_parse_iso(s["created_at"]) for s in steps if isinstance(s.get("created_at"), str)]
    return min(timestamps) if timestamps else None


def _compute_wall_seconds(steps: list[dict[str, Any]]) -> float:
    """Computes total elapsed wall-clock seconds across all timestamped steps."""
    timestamps = [_parse_iso(s["created_at"]) for s in steps if isinstance(s.get("created_at"), str)]
    if len(timestamps) < 2:
        return 0.0
    return round((max(timestamps) - min(timestamps)).total_seconds(), ROUND_DECIMALS)


def _format_step_entry(
    idx: int,
    step: dict[str, Any],
    first_ts: datetime,
    prev_ts: datetime,
) -> tuple[dict[str, Any], datetime, list[str], bool]:
    """Formats one step timeline record and returns updated timestamp, flags, and send_message marker."""
    raw_ts = step.get("created_at")
    cur_ts = _parse_iso(raw_ts) if isinstance(raw_ts, str) else prev_ts
    delta_s = round((cur_ts - prev_ts).total_seconds(), ROUND_DECIMALS)
    elapsed_s = round((cur_ts - first_ts).total_seconds(), ROUND_DECIMALS)
    labels = [_summarize_tool_call(tc) for tc in (step.get("tool_calls") or [])]
    step_flags = _detect_step_anomalies(step)
    record = {
        "step_index": step.get("step_index", idx),
        "elapsed_seconds": elapsed_s,
        "delta_seconds": delta_s,
        "type": step.get("type", ""),
        "input_tokens": int(step.get("input_tokens") or 0),
        "output_tokens": int(step.get("output_tokens") or 0),
        "thinking_chars": len(str(step.get("thinking") or "")),
        "tools": labels,
        "anomalies": step_flags,
    }
    return record, cur_ts, step_flags, ("send_message" in labels)


def _build_timeline_entries(steps: list[dict[str, Any]]) -> tuple[list[dict[str, Any]], list[str], float]:
    """Constructs per-step timeline records, aggregated anomalies, and final synthesis duration."""
    first_ts = _find_first_timestamp(steps)
    if first_ts is None:
        return [], [], 0.0

    prev_ts = first_ts
    timeline: list[dict[str, Any]] = []
    all_anomalies: list[str] = []
    send_msg_start: float | None = None
    final_synth_secs = 0.0

    for idx, step in enumerate(steps):
        rec, prev_ts, flags, is_send = _format_step_entry(idx, step, first_ts, prev_ts)
        if send_msg_start is not None and final_synth_secs == 0.0:
            final_synth_secs = round(rec["elapsed_seconds"] - send_msg_start, ROUND_DECIMALS)
        if is_send:
            send_msg_start = rec["elapsed_seconds"]
        all_anomalies.extend(flags)
        timeline.append(rec)
    return timeline, sorted(set(all_anomalies)), final_synth_secs


def _sum_tokens(steps: list[dict[str, Any]]) -> dict[str, int]:
    """Aggregates input, cache_read, output, and total token counts across steps."""
    in_tok = sum(int(s.get("input_tokens") or 0) for s in steps)
    cache_tok = sum(int(s.get("cache_read_tokens") or 0) for s in steps)
    out_tok = sum(int(s.get("output_tokens") or 0) for s in steps)
    return {"input": in_tok, "cache_read": cache_tok, "output": out_tok, "total": in_tok + cache_tok + out_tok}


def profile_transcript(path: Path, include_steps: bool = True) -> dict[str, Any]:
    """Profiles a subagent transcript JSONL file with zero LLM involvement."""
    steps = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    wall_secs = _compute_wall_seconds(steps)
    tool_secs = _measure_tool_window_seconds(steps)
    llm_secs = round(max(0.0, wall_secs - tool_secs), ROUND_DECIMALS)
    timeline, anomalies, final_synth_secs = _build_timeline_entries(steps)

    result: dict[str, Any] = {
        "conversation_id": path.parent.parent.parent.name,
        "wall_clock_seconds": wall_secs,
        "tool_execution_seconds": tool_secs,
        "llm_inference_and_writing_seconds": llm_secs,
        "final_synthesis_seconds": final_synth_secs,
        "planner_turns": sum(1 for s in steps if s.get("type") == "PLANNER_RESPONSE"),
        "tool_calls": sum(len(s.get("tool_calls") or []) for s in steps),
        "tokens": _sum_tokens(steps),
        "detected_anomalies": anomalies,
    }
    if include_steps:
        result["timeline"] = timeline
    return result


def main(argv: list[str] | None = None) -> int:
    """CLI entry point for generic Agent Skill transcript profiling."""
    parser = argparse.ArgumentParser(description="Profile Agent Skill JSONL transcripts and detect bottlenecks.")
    parser.add_argument("transcripts", nargs="+", help="Paths to transcript_full.jsonl or transcript.jsonl files.")
    parser.add_argument("--summary-only", action="store_true", help="Omit per-step timeline array.")
    args = parser.parse_args(argv)

    profiles = [profile_transcript(Path(p), include_steps=not args.summary_only) for p in args.transcripts]
    print(json.dumps({"profiles": profiles}, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
