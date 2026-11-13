#!/usr/bin/env python3
"""Acoustic aHash Recognition Benchmark on Video Cuts.

Validates acoustic-only (Haitsma-Kalker aHash) video identification, temporal shift accuracy,
and end-to-end headless cvlc subtitle synchronization across authentic media cuts.
Zero display footprint: executes completely offscreen with dummy video/audio drivers.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import time
import urllib.request
from dataclasses import asdict, dataclass
from pathlib import Path

DEFAULT_PORT: int = 8765
DEFAULT_SERVER_URL: str = f"http://127.0.0.1:{DEFAULT_PORT}"
DEFAULT_CUTS_DIR: str = "data/snippets_ahash"
DEFAULT_REPORT_PATH: str = "data/reports/eval_ahash_cuts.json"
MAX_SUBPROCESS_TIMEOUT_SECONDS: float = 15.0
GROUND_TRUTH_CUT_OFFSET_SECONDS: float = 60.0
MILLIS_PER_SEC: float = 1000.0
CERTIFIED_SYNCED_SOURCE: str = "certified_synced"
MAX_PERMISSIBLE_OFFSET_ERROR_MS: float = 25.0


@dataclass
class CutBenchmarkResult:
    cut_file: str
    expected_media: str
    matched_media: str | None
    is_correct_media: bool
    confidence_percent: float
    snr_bits: float
    calculated_shift_ms: float
    ground_truth_shift_ms: float
    shift_error_ms: float
    is_shift_exact: bool
    ffmpeg_ms: float
    ahash_match_ms: float
    api_total_ms: float
    cvlc_success: bool
    cvlc_elapsed_s: float


def find_repo_root() -> Path:
    """Finds repository root dynamically."""
    curr = Path(__file__).resolve().parent
    for p in [curr, *curr.parents]:
        if (p / ".git").is_dir() or (p / "Taskfile.yml").is_file():
            return p
    return Path.cwd()


def http_post_json(url: str, data: dict, timeout: float = 15.0) -> dict:
    """Sends JSON POST request and returns parsed JSON response."""
    payload_bytes = json.dumps(data).encode("utf-8")
    req = urllib.request.Request(
        url,
        data=payload_bytes,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode("utf-8"))


def parse_snr_from_tier(tier_string: str) -> float:
    """Extracts SNR bits value from matching_tier string (e.g. 'Tier aHash Catalog (SNR=30.8 bits)')."""
    m = re.search(r"SNR=([\d\.]+)", tier_string)
    return float(m.group(1)) if m else 0.0


def check_media_title_match(expected_stem: str, matched: str | None) -> bool:
    """Checks whether matched identifier aligns with expected media stem or known moviehash."""
    if not matched:
        return False
    clean_exp = re.sub(r"[\._\-\s]+", "", expected_stem.lower())
    clean_mat = re.sub(r"[\._\-\s]+", "", matched.lower())
    if clean_exp in clean_mat or clean_mat in clean_exp:
        return True
    # Known moviehash alias for Halt and Catch Fire S04E01
    return matched == "4ba3e3f2c751f6f4" and "halt" in expected_stem.lower()


def run_cvlc_single_test(cut_path: Path) -> tuple[bool, float]:
    """Executes single offscreen headless cvlc test using aHash auto-sync."""
    cmd = [
        "cvlc",
        "--vout=dummy",
        "--aout=dummy",
        "--no-audio",
        "--avcodec-hw=none",
        "--extraintf",
        "luaintf",
        "--lua-intf",
        "dbsrt_intf",
        str(cut_path.resolve()),
    ]
    env = os.environ.copy()
    env["DBSRT_AUTOKILL"] = "1"
    env["DBSRT_SEARCH_MODE"] = "ahash"
    env["DISPLAY"] = ""

    t0 = time.monotonic()
    try:
        res = subprocess.run(
            cmd,
            env=env,
            capture_output=True,
            text=True,
            timeout=MAX_SUBPROCESS_TIMEOUT_SECONDS,
            check=False,
        )
        elapsed = time.monotonic() - t0
        output = res.stderr + "\n" + res.stdout
        success = "SUCCESS! Media matched" in output or "VIDEO MATCHED" in output
        return success, round(elapsed, 3)
    except subprocess.TimeoutExpired:
        return False, MAX_SUBPROCESS_TIMEOUT_SECONDS


def evaluate_single_cut(cut_path: Path, server_url: str) -> CutBenchmarkResult | None:
    """Runs aHash REST API query and offscreen cvlc execution on a single video cut."""
    expected_stem = cut_path.name.replace("_cut15s.mp4", "").replace("_mid15s.mp4", "")
    req_payload = {
        "media_path": str(cut_path.resolve()),
        "search_mode": "ahash",
        "language": "auto",
    }
    t0 = time.monotonic()
    try:
        resp = http_post_json(f"{server_url}/v1/search", req_payload)
    except Exception as e:
        print(f"  ❌ API Error on {cut_path.name}: {e}")
        return None

    rtt_ms = (time.monotonic() - t0) * 1000.0
    matched = resp.get("matched_media")
    timing = resp.get("timing", {})
    snr_bits = parse_snr_from_tier(timing.get("matching_tier", ""))

    # Convention: shift = target_ts - ref_ts. A cut extracted at +60s of the reference
    # (probed from its own 0.0s) must move the reference subtitle by -60_000 ms.
    synced_candidate = next(
        (c for c in resp.get("candidates", []) if c.get("source") == CERTIFIED_SYNCED_SOURCE),
        None,
    )
    gt_shift_ms = -GROUND_TRUTH_CUT_OFFSET_SECONDS * MILLIS_PER_SEC
    if synced_candidate is None or synced_candidate.get("shift_ms") is None:
        # No certified_synced candidate: no shift was computed, never count it as exact.
        calc_shift_ms = 0.0
        shift_err = abs(calc_shift_ms - gt_shift_ms)
        is_shift_exact = False
    else:
        calc_shift_ms = float(synced_candidate["shift_ms"])
        shift_err = abs(calc_shift_ms - gt_shift_ms)
        is_shift_exact = shift_err <= MAX_PERMISSIBLE_OFFSET_ERROR_MS
    is_correct = check_media_title_match(expected_stem, matched)

    cvlc_ok, cvlc_time = run_cvlc_single_test(cut_path)

    return CutBenchmarkResult(
        cut_file=cut_path.name,
        expected_media=expected_stem,
        matched_media=matched,
        is_correct_media=is_correct,
        confidence_percent=100.0 if is_correct else 0.0,
        snr_bits=snr_bits,
        calculated_shift_ms=calc_shift_ms,
        ground_truth_shift_ms=gt_shift_ms,
        shift_error_ms=shift_err,
        is_shift_exact=is_shift_exact,
        ffmpeg_ms=round(float(timing.get("ffmpeg_ms", 0.0)), 2),
        ahash_match_ms=round(float(timing.get("phash_match_ms", 0.0)), 2),
        api_total_ms=round(rtt_ms, 2),
        cvlc_success=cvlc_ok,
        cvlc_elapsed_s=cvlc_time,
    )


def print_benchmark_summary(results: list[CutBenchmarkResult], total_duration: float) -> None:
    """Prints comprehensive terminal metrics summary."""
    total_samples = len(results)
    correct_matches = sum(1 for r in results if r.is_correct_media)
    exact_shifts = sum(1 for r in results if r.is_shift_exact)
    cvlc_passes = sum(1 for r in results if r.cvlc_success)

    accuracy_pct = (correct_matches / total_samples * 100.0) if total_samples else 0.0
    exact_shift_pct = (exact_shifts / total_samples * 100.0) if total_samples else 0.0
    cvlc_pct = (cvlc_passes / total_samples * 100.0) if total_samples else 0.0
    avg_api_ms = sum(r.api_total_ms for r in results) / total_samples if total_samples else 0.0
    avg_snr = sum(r.snr_bits for r in results) / total_samples if total_samples else 0.0

    print("\n================================================================================")
    print(" 📊 BILAN MÉTHODOLOGIQUE DU BENCHMARK aHash SUR CUTS RÉELS")
    print("================================================================================")
    print(f"  • Échantillons analysés        : {total_samples} cuts audio")
    print(f"  • Précision Top-1 match        : {accuracy_pct:.1f}% ({correct_matches}/{total_samples})")
    print(f"  • Précision temporelle (≤25ms) : {exact_shift_pct:.1f}% ({exact_shifts}/{total_samples})")
    print(f"  • Marge SNR acoustique moyenne : {avg_snr:.1f} bits")
    print(f"  • Latence moyenne API search   : {avg_api_ms:.1f} ms")
    print(f"  • Taux de succès headless cvlc : {cvlc_pct:.1f}% ({cvlc_passes}/{total_samples})")
    print(f"  • Durée totale d'exécution     : {total_duration:.2f} s")
    print("================================================================================")


def export_benchmark_report(results: list[CutBenchmarkResult], report_file: Path, total_duration: float) -> None:
    """Exports structured audit evaluation data to JSON."""
    total_samples = len(results)
    correct_matches = sum(1 for r in results if r.is_correct_media)
    exact_shifts = sum(1 for r in results if r.is_shift_exact)
    cvlc_passes = sum(1 for r in results if r.cvlc_success)

    report_data = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "total_samples": total_samples,
        "accuracy_percent": round((correct_matches / total_samples * 100.0) if total_samples else 0.0, 2),
        "exact_shift_percent": round((exact_shifts / total_samples * 100.0) if total_samples else 0.0, 2),
        "cvlc_success_percent": round((cvlc_passes / total_samples * 100.0) if total_samples else 0.0, 2),
        "avg_snr_bits": round(sum(r.snr_bits for r in results) / total_samples if total_samples else 0.0, 2),
        "avg_api_ms": round(sum(r.api_total_ms for r in results) / total_samples if total_samples else 0.0, 2),
        "total_duration_s": round(total_duration, 2),
        "details": [asdict(r) for r in results],
    }
    with open(report_file, "w") as f:
        json.dump(report_data, f, indent=2)
    print(f"✓ Rapport d'audit sauvegardé : {report_file}")


def main() -> None:
    parser = argparse.ArgumentParser(description="aHash Acoustic Recognition Benchmark on Video Cuts")
    parser.add_argument("--cuts-dir", default=DEFAULT_CUTS_DIR, help="Path to cuts folder containing audio mp4s")
    parser.add_argument("--server-url", default=DEFAULT_SERVER_URL, help="dbsrt daemon base URL")
    parser.add_argument("--output-json", default=DEFAULT_REPORT_PATH, help="Path to export evaluation JSON report")
    args = parser.parse_args()

    repo_root = find_repo_root()
    cuts_dir = repo_root / args.cuts_dir
    report_file = repo_root / args.output_json
    report_file.parent.mkdir(parents=True, exist_ok=True)

    cut_files = sorted(cuts_dir.glob("*.mp4"))
    if not cut_files:
        print(f"❌ Zero .mp4 cuts found in {cuts_dir}")
        return

    print("================================================================================")
    print(f" 🎧 BANC DE RECONNAISSANCE ACOUSTIQUE aHash ({len(cut_files)} CUTS RÉELS)")
    print("================================================================================")
    print(f"Démon cible   : {args.server_url}")
    print(f"Dossier cuts  : {cuts_dir}")
    print("Mode d'exécution : 100% OFFSCREEN (--vout=dummy --aout=dummy, zéro affichage)")
    print("--------------------------------------------------------------------------------")

    results: list[CutBenchmarkResult] = []
    t_start = time.monotonic()

    for idx, cut_path in enumerate(cut_files, start=1):
        res = evaluate_single_cut(cut_path, args.server_url)
        if res:
            results.append(res)
            icon = "✅" if res.is_correct_media and res.cvlc_success else "⚠️" if res.is_correct_media else "❌"
            print(
                f"  [{idx:2d}/{len(cut_files)}] {icon} Match: {cut_path.name[:32]:<32} -> '{res.matched_media}' | "
                f"SNR: {res.snr_bits:4.1f}b | Shift: {res.calculated_shift_ms:7.1f}ms | API: {res.api_total_ms:5.1f}ms"
            )

    total_duration = time.monotonic() - t_start
    print_benchmark_summary(results, total_duration)
    export_benchmark_report(results, report_file, total_duration)


if __name__ == "__main__":
    main()
