#!/usr/bin/env python3
"""Deterministic, zero-LLM evaluator for pydbsrt-data-access A/B agent benchmarks."""

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
DIGIT_SPACE_RE = re.compile(r"(?<=\d)[\s\u00a0\u202f]+(?=\d)|(?<=\d),(?=\d{3}\b)")
ROUND_DECIMALS: int = 2

GROUND_TRUTH_CHECKS: tuple[tuple[str, str], ...] = (
    ("q1_local_ahash_538", r"\b538\b"),
    ("q1_rpi4_ahash_0", r"rpi4[^\n]*\b0\b"),
    ("q1_o2switch_ahash_2548", r"\b2548\b"),
    ("q1_o2switch_most_complete", r"o2switch"),
    ("q2_catalog_not_up_to_date", r"(stale_local|pas à jour|non\b|en retard)"),
    ("q2_local_size_241013560", r"\b241013560\b"),
    ("q2_remote_size_395529096", r"\b395529096\b"),
    ("q2_local_date_2026_11_11", r"2026-11-11"),
    ("q2_remote_date_2026_11_14", r"2026-11-14"),
    ("q3_freebox_disque_1", r"disque_1\b"),
    ("q3_freebox_disque_2", r"disque_2\b"),
    ("q3_freebox_disque_1_1", r"disque_1_1\b"),
    ("q3_rpi4_mount_freebox", r"/mnt/freebox/disque_1_1"),
    ("q4_andor_s01e01_snippet", r"andor\.s01e01\.1080p\.web\.h264-kogi_mid15s\.mp4"),
    ("q4_parent_hash_and_21_subs", r"(5cc955f9c40a26756418d6aa198ea273|\b21\b)"),
    ("q4_s01e05_s01e06_uncoupled", r"s01e05.*s01e06"),
)


def _parse_iso(ts: str) -> datetime:
    """Parses ISO-8601 timestamp into timezone-aware datetime."""
    return datetime.fromisoformat(ts.replace("Z", "+00:00"))


def _extract_send_message_arg(tc: dict[str, Any]) -> str | None:
    """Returns Message argument if tool call is send_message, else None."""
    if (tc.get("name") or tc.get("tool_name")) != "send_message":
        return None
    args = tc.get("arguments") or tc.get("args") or {}
    if isinstance(args, str):
        args = json.loads(args)
    return str(args.get("Message", ""))


def _extract_final_report(steps: list[dict[str, Any]]) -> str:
    """Extracts the final answer sent via send_message (or last planner content)."""
    for step in reversed(steps):
        for tc in step.get("tool_calls") or []:
            msg = _extract_send_message_arg(tc)
            if msg is not None:
                return msg
    for step in reversed(steps):
        if step.get("type") == "PLANNER_RESPONSE" and step.get("content"):
            return str(step["content"])
    return ""


def _measure_tool_window_seconds(steps: list[dict[str, Any]]) -> float:
    """Computes union of [Created At, Completed At] intervals across all tool executions."""
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


def grade_response_text(report: str) -> dict[str, Any]:
    """Evaluates an agent answer deterministically against the 16 validated assertions."""
    normalized = DIGIT_SPACE_RE.sub("", report).lower()
    passed: list[str] = []
    failed: list[str] = []
    for check_id, pattern in GROUND_TRUTH_CHECKS:
        if re.search(pattern, normalized, flags=re.DOTALL):
            passed.append(check_id)
        else:
            failed.append(check_id)
    total = len(GROUND_TRUTH_CHECKS)
    return {
        "score": f"{len(passed)}/{total}",
        "passed_count": len(passed),
        "total_checks": total,
        "all_passed": len(failed) == 0,
        "failed_checks": failed,
    }


def evaluate_transcript(path: Path) -> dict[str, Any]:
    """Computes exact timing, token telemetry, and deterministic accuracy score."""
    steps = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    timestamps = [_parse_iso(s["created_at"]) for s in steps if isinstance(s.get("created_at"), str)]
    wall_secs = (
        round((max(timestamps) - min(timestamps)).total_seconds(), ROUND_DECIMALS) if len(timestamps) >= 2 else 0.0
    )
    tool_secs = _measure_tool_window_seconds(steps)
    llm_secs = round(max(0.0, wall_secs - tool_secs), ROUND_DECIMALS)

    in_tok = sum(int(s.get("input_tokens") or 0) for s in steps)
    cache_tok = sum(int(s.get("cache_read_tokens") or 0) for s in steps)
    out_tok = sum(int(s.get("output_tokens") or 0) for s in steps)
    planner_turns = sum(1 for s in steps if s.get("type") == "PLANNER_RESPONSE")
    tool_calls = sum(len(s.get("tool_calls") or []) for s in steps)

    report = _extract_final_report(steps)
    grading = grade_response_text(report)

    return {
        "conversation_id": path.parent.parent.parent.name,
        "wall_clock_seconds": wall_secs,
        "llm_inference_and_writing_seconds": llm_secs,
        "tool_execution_seconds": tool_secs,
        "planner_turns": planner_turns,
        "tool_calls": tool_calls,
        "tokens": {
            "input": in_tok,
            "cache_read": cache_tok,
            "output": out_tok,
            "total": in_tok + cache_tok + out_tok,
        },
        "report_chars": len(report),
        "grading": grading,
    }


def main(argv: list[str] | None = None) -> int:
    """CLI entry point evaluating one or more subagent transcript JSONL files."""
    parser = argparse.ArgumentParser(description="Deterministic evaluator for pydbsrt-data-access benchmarks.")
    parser.add_argument("transcripts", nargs="+", help="Paths to transcript_full.jsonl files.")
    args = parser.parse_args(argv)

    results = [evaluate_transcript(Path(p)) for p in args.transcripts]
    print(json.dumps({"evaluations": results}, indent=2, ensure_ascii=False))
    return 0 if all(r["grading"]["all_passed"] for r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
