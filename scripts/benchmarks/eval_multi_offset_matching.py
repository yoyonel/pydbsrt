#!/usr/bin/env python3
"""Evaluation Harness comparing Mono-Offset (t=50%) vs Multi-Offset Spatiotemporal Consensus.

Evaluates recognition accuracy, false collision elimination rate, and latency
across difficult anime/series snippets with shared intros, eyecatches, and jingles.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from pathlib import Path

DEFAULT_DBPHASH = "data/catalog.dbphash"
DEFAULT_BENCHMARK_TARGETS = [
    "data/snippets/Demon_Slayer_–_Saison_2_Episode_10_VOSTFR_mid15s.mp4",
    "data/snippets/Demon_Slayer_–_Saison_2_Episode_11_VOSTFR_mid15s.mp4",
    "data/snippets/Demon_Slayer_–_Saison_2_Episode_12_VOSTFR_mid15s.mp4",
    "data/snippets/Demon_Slayer_–_Saison_2_Episode_13_VOSTFR_mid15s.mp4",
    "data/snippets/Demon_Slayer_–_Saison_2_Episode_14_VOSTFR_mid15s.mp4",
    "data/snippets/Demon_Slayer_–_Saison_2_Episode_15_VOSTFR_mid15s.mp4",
    "data/snippets/Demon_Slayer_–_Saison_2_Episode_16_VOSTFR_mid15s.mp4",
    "data/snippets/Demon_Slayer_–_Saison_2_Episode_17_VOSTFR_mid15s.mp4",
    "data/snippets/Demon_Slayer_–_Saison_2_Episode_18_VOSTFR_mid15s.mp4",
]


def find_repo_root() -> Path:
    """Dynamically locates repository root with fallback to ~/dbsrt."""
    current = Path(__file__).resolve().parent
    for p in [current, *current.parents]:
        if (p / ".git").is_dir() or (p / "Taskfile.yml").is_file():
            return p
    appliance = Path.home() / "dbsrt"
    if appliance.is_dir():
        return appliance
    return Path.cwd()


@dataclass
class EvalRunResult:
    media_path: str
    media_stem: str
    mono_match_found: bool
    mono_matched_title: str
    mono_matched_id: int
    mono_confidence: float
    mono_timer_sec: float
    multi_match_found: bool
    multi_matched_title: str
    multi_matched_id: int
    multi_confidence: float
    multi_windows: str
    multi_coherent: bool
    multi_timer_sec: float
    collision_eliminated: bool


def _parse_multi_csv(parts: list[str], elapsed: float) -> dict:
    found = parts[0] == "True"
    clean_title = parts[2] if len(parts) > 2 else ""
    m_id = int(parts[3]) if len(parts) > 3 and parts[3].isdigit() else 0
    windows = parts[4] if len(parts) > 4 else "0/0"
    coherent = parts[5] == "True" if len(parts) > 5 else False
    conf = float(parts[7]) if len(parts) > 7 else 0.0
    return {
        "found": found,
        "title": clean_title,
        "media_id": m_id,
        "confidence": conf,
        "windows": windows,
        "coherent": coherent,
        "timer": elapsed,
    }


def _parse_mono_csv(parts: list[str], elapsed: float) -> dict:
    found = parts[0] == "True"
    clean_title = parts[2] if len(parts) > 2 else ""
    m_id = int(parts[3]) if len(parts) > 3 and parts[3].isdigit() else 0
    conf = float(parts[6]) if len(parts) > 6 else 0.0
    return {
        "found": found,
        "title": clean_title,
        "media_id": m_id,
        "confidence": conf,
        "windows": "1/1",
        "coherent": True,
        "timer": elapsed,
    }


def run_dbsrt_recognize(
    dbsrt_bin: Path,
    media_path: Path,
    dbphash_path: Path,
    multi_window: bool,
) -> dict:
    cmd = [
        str(dbsrt_bin),
        "recognize-media",
        "--media",
        str(media_path),
        "--dbphash",
        str(dbphash_path),
        "--output-format",
        "CSV",
        "--limit",
        "1",
    ]
    if multi_window:
        cmd.append("--multi-window")

    t0 = time.perf_counter()
    proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
    elapsed = time.perf_counter() - t0

    if proc.returncode != 0 or not proc.stdout.strip():
        return {
            "found": False,
            "title": "NONE",
            "media_id": 0,
            "confidence": 0.0,
            "windows": "0/0",
            "coherent": False,
            "timer": elapsed,
        }

    line = proc.stdout.strip().split("\n")[0]
    parts = line.split(",")
    if multi_window:
        return _parse_multi_csv(parts, elapsed)
    return _parse_mono_csv(parts, elapsed)


def _eval_single_media(bin_path: Path, dbphash_path: Path, path: Path) -> EvalRunResult:
    stem = path.stem
    mono = run_dbsrt_recognize(bin_path, path, dbphash_path, multi_window=False)
    multi = run_dbsrt_recognize(bin_path, path, dbphash_path, multi_window=True)

    is_mono_collision = (not mono["found"]) and (mono["confidence"] >= 90.0)
    collision_eliminated = is_mono_collision and (multi["confidence"] < 50.0 or not multi["coherent"])

    print(
        f"📹 {stem[:45]:<45} | "
        f"Mono: {mono['confidence']:5.1f}% ({'MATCH' if mono['found'] else 'COLLISION':9}) | "
        f"Multi: {multi['confidence']:5.1f}% (win={multi['windows']}, coh={str(multi['coherent']):5}) | "
        f"Slashing: {'ELIMINATED' if collision_eliminated else 'PASS'}"
    )

    return EvalRunResult(
        media_path=str(path),
        media_stem=stem,
        mono_match_found=mono["found"],
        mono_matched_title=mono["title"],
        mono_matched_id=mono["media_id"],
        mono_confidence=mono["confidence"],
        mono_timer_sec=mono["timer"],
        multi_match_found=multi["found"],
        multi_matched_title=multi["title"],
        multi_matched_id=multi["media_id"],
        multi_confidence=multi["confidence"],
        multi_windows=multi["windows"],
        multi_coherent=multi["coherent"],
        multi_timer_sec=multi["timer"],
        collision_eliminated=collision_eliminated,
    )


def _summarize_and_save(results: list[EvalRunResult], out_path: Path) -> None:
    collisions_in_mono = sum(1 for r in results if not r.mono_match_found and r.mono_confidence >= 90.0)
    eliminated = sum(1 for r in results if r.collision_eliminated)

    print("=" * 80)
    print("📊 SUMMARY:")
    print(f"  Total Evaluated: {len(results)}")
    print(f"  Mono-Window False Collisions (>=90%): {collisions_in_mono}")
    pct = (eliminated / collisions_in_mono * 100.0) if collisions_in_mono else 100.0
    print(f"  Collisions Eliminated by Multi-Window: {eliminated} / {collisions_in_mono} ({pct:.1f}%)")

    out_path.parent.mkdir(parents=True, exist_ok=True)
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump([asdict(r) for r in results], f, indent=2)
    print(f"💾 Report saved to {out_path}")


def parse_cli_args(repo_root: Path) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Benchmark Mono-Window vs Multi-Window Spatiotemporal Recognition")
    parser.add_argument(
        "--dbphash",
        type=Path,
        default=repo_root / DEFAULT_DBPHASH,
        help="Path to .dbphash catalog",
    )
    default_bin = (
        repo_root / "target/release/dbsrt"
        if (repo_root / "target/release/dbsrt").exists()
        else repo_root / "target/debug/dbsrt"
    )
    parser.add_argument(
        "--bin",
        type=Path,
        default=default_bin,
        help="Path to dbsrt binary",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=repo_root / "data/reports/eval_multi_offset_matching.json",
        help="Output evaluation report JSON file",
    )
    parser.add_argument(
        "--check",
        "--dry-run",
        action="store_true",
        help="Inspect catalog, binary, and target files without running evaluation",
    )
    parser.add_argument(
        "files",
        nargs="*",
        help="Optional custom media video files to evaluate",
    )
    return parser.parse_args()


def _resolve_eval_targets(repo_root: Path, custom_files: list[str]) -> list[Path]:
    """Resolves target video paths against repo root."""
    if custom_files:
        return [Path(f) if Path(f).is_absolute() else (repo_root / f) for f in custom_files]
    return [repo_root / f for f in DEFAULT_BENCHMARK_TARGETS]


def main() -> int:
    repo_root = find_repo_root()
    args = parse_cli_args(repo_root)

    dbphash_path = args.dbphash if args.dbphash.is_absolute() else (repo_root / args.dbphash)
    bin_path = args.bin if args.bin.is_absolute() else (repo_root / args.bin)
    out_path = args.out if args.out.is_absolute() else (repo_root / args.out)

    targets = _resolve_eval_targets(repo_root, args.files)
    valid_targets = [p for p in targets if p.exists()]

    if args.check:
        print("=== DBSrt Multi-Offset Matching Evaluation Inspection ===")
        print(f"✓ REPO_ROOT     : {repo_root}")
        print(f"✓ Catalog       : {dbphash_path} (exists: {dbphash_path.exists()})")
        print(f"✓ DBSrt binary  : {bin_path} (exists: {bin_path.exists()})")
        print(f"✓ Target files  : {len(valid_targets)}/{len(targets)} exist")
        print(f"✓ Output report : {out_path}")
        return 0

    if not valid_targets:
        print("❌ No valid media targets found to evaluate.", file=sys.stderr)
        return 1

    print(f"🚀 Evaluating {len(valid_targets)} target media files...")
    print(f"📦 Catalog: {dbphash_path} (size: {dbphash_path.stat().st_size / 1024 / 1024:.1f} MB)")
    print("=" * 80)

    results = [_eval_single_media(bin_path, dbphash_path, p) for p in valid_targets]
    _summarize_and_save(results, out_path)
    return 0


if __name__ == "__main__":
    sys.exit(main())
