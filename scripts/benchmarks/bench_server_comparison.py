#!/usr/bin/env python3
"""Objective comparative benchmark measuring dbsrt-server matching latency:
Before refactoring (Direct pointer) vs After refactoring (ArcSwap atomic pointer).
"""

from __future__ import annotations

import json
import statistics
import subprocess
import sys
import time
import urllib.request
from pathlib import Path


def wait_for_server(url: str, timeout: float = 5.0) -> bool:
    t0 = time.time()
    while time.time() - t0 < timeout:
        try:
            req = urllib.request.Request(f"{url}/v1/health")
            with urllib.request.urlopen(req, timeout=1.0) as resp:
                if resp.status == 200:
                    return True
        except Exception:
            time.sleep(0.05)
    return False


def run_benchmark_for_binary(
    binary_path: str,
    port: int,
    queries: list[dict],
    n_warmup: int = 50,
) -> list[float]:
    print(f"\n--- Testing binary: {binary_path} on port {port} ---", flush=True)
    cmd = [
        binary_path,
        "--host",
        "127.0.0.1",
        "--port",
        str(port),
        "--phash-dir",
        "tests/data",
        "--subtitles-dir",
        "data/SRT",
    ]
    proc = subprocess.Popen(
        cmd,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    base_url = f"http://127.0.0.1:{port}"

    try:
        if not wait_for_server(base_url):
            raise RuntimeError(f"Server {binary_path} failed to start on port {port}")

        # Warm-up phase
        warmup_payload = json.dumps(queries[0]).encode("utf-8")
        for _ in range(n_warmup):
            req = urllib.request.Request(
                f"{base_url}/v1/recognize",
                data=warmup_payload,
                headers={"Content-Type": "application/json"},
                method="POST",
            )
            with urllib.request.urlopen(req, timeout=5.0) as resp:
                assert resp.status == 200

        # Measurement phase
        latencies_us: list[float] = []
        for q in queries:
            payload = json.dumps(q).encode("utf-8")
            req = urllib.request.Request(
                f"{base_url}/v1/recognize",
                data=payload,
                headers={"Content-Type": "application/json"},
                method="POST",
            )
            t_start = time.perf_counter()
            with urllib.request.urlopen(req, timeout=5.0) as resp:
                assert resp.status == 200
            t_elapsed = (time.perf_counter() - t_start) * 1_000_000.0  # in microseconds
            latencies_us.append(t_elapsed)

        return latencies_us

    finally:
        proc.terminate()
        proc.wait()


def compute_stats(latencies: list[float]) -> dict[str, float]:
    sorted_l = sorted(latencies)
    n = len(sorted_l)
    return {
        "mean": statistics.mean(sorted_l),
        "median": statistics.median(sorted_l),
        "p90": sorted_l[int(n * 0.90)],
        "p95": sorted_l[int(n * 0.95)],
        "p99": sorted_l[int(n * 0.99)],
        "min": sorted_l[0],
        "max": sorted_l[-1],
        "stdev": statistics.stdev(sorted_l) if n > 1 else 0.0,
    }


def main():
    bin_before = "/tmp/dbsrt-server-before"
    bin_after = "/tmp/dbsrt-server-after"

    if not Path(bin_before).exists() or not Path(bin_after).exists():
        print(f"Error: missing binaries: {bin_before} or {bin_after}", file=sys.stderr)
        sys.exit(1)

    # Prepare realistic 15-frame query payload
    # 15 hashes representing a 0.6s to 1s snippet
    test_hashes = [
        "0xbf6b0390611f6ccc",
        "0xbf6b0390611f6ccc",
        "0xcc9c3b6326585bc9",
        "0xcc9c3b6326585bc9",
        "0xcc9c3b6326585bc9",
        "0x9e3963c633a699c6",
        "0x9e3963c633a699c6",
        "0x9e3963c633a699c6",
        "0xe19c3b6326585bc9",
        "0xe19c3b6326585bc9",
        "0xbf6b0390611f6ccc",
        "0xbf6b0390611f6ccc",
        "0xcc9c3b6326585bc9",
        "0xcc9c3b6326585bc9",
        "0x9e3963c633a699c6",
    ]

    n_samples = 1000
    queries = [
        {
            "media_path": "query_sample.mp4",
            "query_fingerprint": test_hashes,
            "fps": 25.0,
            "language": "fr",
            "search_distance": 6,
            "nb_seconds": 1.0,
        }
        for _ in range(n_samples)
    ]

    print("================================================================================")
    print(" 🔬 EMPIRICAL SERVER MATCHING COMPARISON: BEFORE vs AFTER REFACTORING")
    print(f" Samples: {n_samples} HTTP POST /v1/recognize queries under identical conditions")
    print("================================================================================")

    lat_before = run_benchmark_for_binary(bin_before, 8798, queries)
    lat_after = run_benchmark_for_binary(bin_after, 8799, queries)

    s_before = compute_stats(lat_before)
    s_after = compute_stats(lat_after)

    print("\n================================================================================")
    print(" 📊 COMPARATIVE LATENCY SUMMARY (HTTP roundtrip in microseconds µs)")
    print("================================================================================")
    print(f"{'Métrique':<15} | {'Avant (Direct)':<18} | {'Après (ArcSwap)':<18} | {'Delta':<15} | {'Impact':<10}")
    print("-" * 84)

    for metric in ["median", "mean", "p90", "p95", "p99", "min", "stdev"]:
        v_b = s_before[metric]
        v_a = s_after[metric]
        delta = v_a - v_b
        pct = (delta / v_b) * 100.0 if v_b > 0 else 0.0
        sign = "+" if delta >= 0 else ""
        print(
            f"{metric.upper():<15} | {v_b:>14.2f} µs | {v_a:>14.2f} µs | {sign}{delta:>10.2f} µs | {sign}{pct:>6.2f}%"
        )

    print("================================================================================")


if __name__ == "__main__":
    main()
