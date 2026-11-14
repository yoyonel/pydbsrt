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

FLASH_LAN_V2_CHECKS: tuple[tuple[str, str], ...] = (
    ("q1_ahash_file", r"01f6f860fd5d63ed\.86\.1fps\.ahash"),
    ("q1_moviehash", r"01f6f860fd5d63ed"),
    ("q1_duration_2462", r"2462[.,]69"),
    ("q1_frames_212099", r"\b212099\b"),
    ("q2_disk_1_1", r"disque_1_1"),
    ("q2_boite_noire_mkv", r"boite noire \(2021\)\[1080p\]\(thriller , drame, pierre niney\)\.mkv"),
    ("q2_size_9387647743", r"\b9387647743\b"),
    ("q2_video_h264_1920x800", r"(h264|h\.264).*1920\s*[x×]\s*800|1920\s*[x×]\s*800.*(h264|h\.264)"),
    ("q2_audio_dts_6ch", r"dts.*(\b6\b|5\.1)|(\b6\b|5\.1).*dts"),
    (
        "q3_tvshow_subpath",
        r"locke and key \[1080p\]/s01/locke\.and\.key\.s01e01\.welcome\.to\.matheson\.1080p\.nf\.web-dl\.ddp5\.1\.x264-ntg\.mkv",
    ),
    ("q3_size_2455763846", r"\b2455763846\b"),
    ("q3_moviehash_or_phash", r"ffb6fed130ab99c8"),
    ("q3_coupled_26_subs", r"\b26\b"),
    ("q4_local_ahash_2503", r"\b2503\b"),
    ("q4_o2switch_ahash_2548", r"\b2548\b"),
    ("q4_catalog_4974_up_to_date", r"(up_to_date|à jour).*\b4974\b|\b4974\b.*(up_to_date|à jour)"),
)

FLASH_NOMAD_V2_CHECKS: tuple[tuple[str, str], ...] = (
    (
        "q1_ahash_tre_piani",
        r"tre piani \(2021\)\[multi\]\[1080p\]\(comédie dramatique italienne\)\.mkv\.86\.1fps\.ahash",
    ),
    ("q1_size_4760912", r"\b4760912\b"),
    ("q1_duration_6909", r"6909[.,]63"),
    ("q1_phash_002835455e305309", r"002835455e305309\.24\.0fps\.phash"),
    ("q2_disk_1", r"disque_1\b"),
    ("q2_arcane_mkv", r"arcane \(2021\) - s01e01 - welcome to the playground \(1080p nf web-dl x265 t3nzin\)\.mkv"),
    ("q2_size_1188423670", r"\b1188423670\b"),
    ("q2_video_hevc_1920x816", r"(hevc|h\.265|x265).*1920\s*[x×]\s*816|1920\s*[x×]\s*816.*(hevc|h\.265|x265)"),
    ("q2_audio_eac3_6ch", r"eac3"),
    ("q2_subs_33", r"\b33\b"),
    (
        "q3_belfast_path",
        r"belfast \(2021\)\[1080p\]\(fr srt, mkv\)/belfast\.2021\.1080p\.amzn\.webrip\.ddp5\.1\.atmos\.x264-nogrp\.mkv",
    ),
    ("q3_size_5505707453", r"\b5505707453\b"),
    ("q3_phash_9f4a607676fe9342", r"9f4a607676fe9342\.24\.0fps\.phash"),
    ("q3_ahash_9f4a607676fe9342", r"9f4a607676fe9342\.86\.1fps\.ahash"),
    ("q4_o2switch_https_channel", r"o2switch_https_(bounce|api)|https_o2switch|https_api"),
    ("q4_o2switch_4974_medias", r"\b4974\b"),
)

BENCHMARK_SUITES: dict[str, tuple[tuple[str, str], ...]] = {
    "v1": GROUND_TRUTH_CHECKS,
    "flash_lan_v2": FLASH_LAN_V2_CHECKS,
    "flash_nomad_v2": FLASH_NOMAD_V2_CHECKS,
}


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


def grade_response_text(report: str, suite: str = "v1") -> dict[str, Any]:
    """Evaluates an agent answer deterministically against the selected benchmark suite."""
    checks = BENCHMARK_SUITES[suite]
    normalized = DIGIT_SPACE_RE.sub("", report).lower()
    passed: list[str] = []
    failed: list[str] = []
    for check_id, pattern in checks:
        if re.search(pattern, normalized, flags=re.DOTALL):
            passed.append(check_id)
        else:
            failed.append(check_id)
    total = len(checks)
    return {
        "suite": suite,
        "score": f"{len(passed)}/{total}",
        "passed_count": len(passed),
        "total_checks": total,
        "all_passed": len(failed) == 0,
        "failed_checks": failed,
    }


def evaluate_transcript(path: Path, suite: str = "v1") -> dict[str, Any]:
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
    grading = grade_response_text(report, suite=suite)

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
    parser.add_argument(
        "--suite",
        choices=sorted(BENCHMARK_SUITES.keys()),
        default="v1",
        help="Deterministic assertion suite to evaluate against (default: v1).",
    )
    parser.add_argument("transcripts", nargs="+", help="Paths to transcript_full.jsonl files.")
    args = parser.parse_args(argv)

    results = [evaluate_transcript(Path(p), suite=args.suite) for p in args.transcripts]
    print(json.dumps({"evaluations": results}, indent=2, ensure_ascii=False))
    return 0 if all(r["grading"]["all_passed"] for r in results) else 1


if __name__ == "__main__":
    sys.exit(main())
