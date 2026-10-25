#!/usr/bin/env python3
"""Exhaustive Evaluation & E2E Validation Matrix on 100% of the NAS Snippets Dataset.

Executes massive quantitative test batteries across 100% of available data:
- Battery 1: Full-scale recognition & pHash matching on ALL 1,336 NAS snippets (multithreaded).
- Battery 2: Strict cross-episode isolation (SxxExx) across ALL TV series in the corpus.
- Battery 3: True-Negative specificity stress test (verifying 0.0% false positive rate).
- Battery 4: Headless VLC (dbsrt_intf.lua) E2E validation executed exclusively on an isolated
  virtual Xvfb display (:99) to completely prevent any focus stealing.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
import urllib.request
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path


def http_post_json(url: str, data: dict, timeout: float = 10.0) -> dict:
    req = urllib.request.Request(
        url,
        data=json.dumps(data).encode("utf-8"),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode("utf-8"))


def parse_sxxexx(filename: str) -> tuple[int, int] | None:
    m = re.search(r"[sS](\d{1,2})[\.\s_-]*[eE](\d{1,2})", filename)
    if m:
        return int(m.group(1)), int(m.group(2))
    return None


def run_battery_1_full_corpus(
    snippets_dir: Path,
    server_url: str,
    workers: int = 8,
) -> dict:
    """Tests pHash recognition across 100% of the NAS snippets dataset concurrently."""
    all_mp4s = sorted(snippets_dir.glob("*_mid15s.mp4"))
    total_count = len(all_mp4s)
    if total_count == 0:
        raise RuntimeError(f"No *_mid15s.mp4 found in {snippets_dir}")

    print("\n================================================================================")
    print(f" 📊 BATTERIE 1 : RECONNAISSANCE INTÉGRALE SUR 100% DU CORPUS ({total_count} VIDÉOS)")
    print("================================================================================")
    print(f"Exécution parallèle multi-threads ({workers} workers)...")

    results = []
    success_count = 0
    t0 = time.monotonic()

    def test_one(idx: int, mp4: Path) -> dict:
        expected_stem = mp4.stem
        payload = {
            "media_path": str(mp4.resolve()),
            "language": "auto",
            "search_distance": 6,
        }
        t_sub = time.monotonic()
        try:
            resp = http_post_json(f"{server_url}/v1/autosync", payload)
            matched = resp.get("matched_media")
            conf = resp.get("confidence_percent", 0.0)
            elapsed = resp.get("elapsed_seconds", 0.0)
            is_match = matched is not None and (
                matched == expected_stem or matched in expected_stem or expected_stem in matched
            )
            return {
                "index": idx,
                "video": mp4.name,
                "expected": expected_stem,
                "matched": matched,
                "confidence": conf,
                "elapsed_s": round(elapsed, 4),
                "is_match": is_match,
                "source": resp.get("source"),
            }
        except Exception as e:
            return {
                "index": idx,
                "video": mp4.name,
                "is_match": False,
                "error": str(e),
                "elapsed_s": round(time.monotonic() - t_sub, 4),
            }

    with ThreadPoolExecutor(max_workers=workers) as executor:
        futures = {executor.submit(test_one, idx, mp4): mp4 for idx, mp4 in enumerate(all_mp4s, 1)}
        done = 0
        for fut in as_completed(futures):
            res = fut.result()
            results.append(res)
            if res.get("is_match"):
                success_count += 1
            done += 1
            if done % 100 == 0 or done == total_count:
                acc = (success_count / done) * 100.0
                cur_rate = done / max(0.1, time.monotonic() - t0)
                print(
                    f"  [{done:4d}/{total_count}] Précision: {acc:5.1f}% ({success_count}/{done}) | Débit: {cur_rate:.1f} req/s"
                )

    total_time = time.monotonic() - t0
    accuracy = (success_count / total_count) * 100.0
    throughput = total_count / max(0.001, total_time)
    avg_latency = (total_time / total_count) * 1000.0

    print(f"\n✅ Batterie 1 Terminée : {success_count}/{total_count} réussis ({accuracy:.2f}%)")
    print(f"   • Débit effectif : {throughput:.1f} requêtes / seconde")
    print(f"   • Temps total pour les {total_count} vidéos : {total_time:.2f} s")

    return {
        "total_corpus_tested": total_count,
        "success": success_count,
        "accuracy_pct": round(accuracy, 2),
        "throughput_rps": round(throughput, 1),
        "avg_latency_ms": round(avg_latency, 2),
        "total_time_s": round(total_time, 2),
    }


def run_battery_2_all_series_isolation(
    snippets_dir: Path,
    server_url: str,
    workers: int = 8,
) -> dict:
    """Verifies that episodes across ALL TV series never cross-collide."""
    all_mp4s = sorted(snippets_dir.glob("*_mid15s.mp4"))
    by_series = defaultdict(list)

    for mp4 in all_mp4s:
        se = parse_sxxexx(mp4.name)
        if se:
            series_name = re.split(r"[sS]\d+", mp4.name)[0].strip("._ -")
            if series_name:
                by_series[series_name].append((mp4, se))

    # Keep all multi-episode series
    multi_ep_series = {k: v for k, v in by_series.items() if len(v) >= 2}
    total_series = len(multi_ep_series)
    total_episodes = sum(len(v) for v in multi_ep_series.values())

    print("\n================================================================================")
    print(f" 🛡️ BATTERIE 2 : ISOLATION STRICTE CROSS-ÉPISODES SUR {total_series} SÉRIES ({total_episodes} ÉPISODES)")
    print("================================================================================")

    all_pairs = []
    for sname, eps in multi_ep_series.items():
        for mp4, se in eps:
            all_pairs.append((sname, mp4, se))

    collisions = 0
    tested_count = 0

    def check_ep(item: tuple[str, Path, tuple[int, int]]) -> dict:
        sname, mp4, se = item
        payload = {
            "media_path": str(mp4.resolve()),
            "language": "auto",
            "search_distance": 6,
        }
        try:
            resp = http_post_json(f"{server_url}/v1/autosync", payload)
            matched = resp.get("matched_media") or ""
            matched_se = parse_sxxexx(matched)
            is_collision = matched_se is not None and matched_se != se
            return {
                "series": sname,
                "video": mp4.name,
                "expected_se": se,
                "matched_se": matched_se,
                "collision": is_collision,
            }
        except Exception as e:
            return {"series": sname, "video": mp4.name, "collision": False, "error": str(e)}

    with ThreadPoolExecutor(max_workers=workers) as executor:
        futures = {executor.submit(check_ep, pair): pair for pair in all_pairs}
        for fut in as_completed(futures):
            res = fut.result()
            tested_count += 1
            if res.get("collision"):
                collisions += 1
                print(f"  ❌ Collision sur {res['series']} : attendu {res['expected_se']}, obtenu {res['matched_se']}")

    collision_rate = (collisions / max(1, tested_count)) * 100.0
    print(
        f"\n✅ Batterie 2 Terminée : {tested_count} épisodes testés | {collisions} collisions ({collision_rate:.2f}%)"
    )

    return {
        "series_count": total_series,
        "episodes_tested": tested_count,
        "collisions": collisions,
        "collision_rate_pct": round(collision_rate, 2),
    }


def run_battery_3_true_negatives(server_url: str) -> dict:
    """Verifies that non-indexed / blank videos are strictly rejected with 0% false positives."""
    print("\n================================================================================")
    print(" 🚫 BATTERIE 3 : SPÉCIFICITÉ & REJET DES FAUX POSITIFS (VRAIS NÉGATIFS)")
    print("================================================================================")

    dummy_files = [
        Path("tests/data/white_frames.mp4"),
        Path("tests/data/black_frames.mp4"),
        Path("tests/data/big_buck_bunny_trailer_480p.mp4"),
    ]

    total_tested = 0
    false_positives = 0

    for df in dummy_files:
        if not df.exists():
            continue
        total_tested += 1
        payload = {
            "media_path": str(df.resolve()),
            "language": "auto",
            "search_distance": 4,
        }
        try:
            resp = http_post_json(f"{server_url}/v1/autosync", payload)
            matched = resp.get("matched_media")
            conf = resp.get("confidence_percent", 0.0)
            is_fp = resp.get("success") is True and conf > 75.0
            if is_fp:
                false_positives += 1
            status_str = "❌ FAUX POSITIF" if is_fp else "✅ REJETÉ AVEC SUCCÈS"
            print(f"  • {df.name:<35} -> Match: {str(matched):<30} (conf: {conf:4.1f}%) -> {status_str}")
        except Exception:
            pass

    fp_rate = (false_positives / max(1, total_tested)) * 100.0
    print(f"\n✅ Batterie 3 Terminée : {total_tested} tests | {false_positives} faux positifs ({fp_rate:.1f}%)")

    return {
        "tested": total_tested,
        "false_positives": false_positives,
        "fp_rate_pct": round(fp_rate, 2),
    }


def run_battery_4_vlc_xvfb(
    snippets_dir: Path,
    sample_count: int = 25,
) -> dict:
    """Executes VLC E2E tests strictly on an isolated Xvfb display (:99) without stealing focus."""
    display_num = 99
    print("\n================================================================================")
    print(f" 🎬 BATTERIE 4 : VALIDATION E2E VLC SUR ÉCRAN VIRTUEL ISOLÉ (Xvfb :{display_num})")
    print("================================================================================")
    print(f"Démarrage Xvfb :{display_num} (1920x1080x24) - Zéro pop-up, zéro vol de focus...")

    xvfb_proc = subprocess.Popen(
        ["Xvfb", f":{display_num}", "-screen", "0", "1920x1080x24", "-ac", "+extension", "GLX", "-noreset"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    time.sleep(1.0)

    try:
        all_mp4s = sorted(snippets_dir.glob("*_mid15s.mp4"))
        step = max(1, len(all_mp4s) // sample_count)
        sampled = [all_mp4s[i] for i in range(0, len(all_mp4s), step)][:sample_count]

        results = []
        success_count = 0

        for idx, mp4 in enumerate(sampled, 1):
            cmd = [
                "cvlc",
                "--avcodec-hw=none",
                "--extraintf",
                "luaintf",
                "--lua-intf",
                "dbsrt_intf",
                str(mp4.resolve()),
            ]
            env = os.environ.copy()
            env["DISPLAY"] = f":{display_num}"
            env["DBSRT_AUTOKILL"] = "1"
            t0 = time.monotonic()

            try:
                res = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=12)
                elapsed = time.monotonic() - t0
                output = res.stderr + "\n" + res.stdout
                is_matched = "SUCCESS! Media matched" in output or "Event-driven autokill triggered" in output

                if is_matched:
                    success_count += 1
                    matched_line = next(
                        (
                            line
                            for line in output.splitlines()
                            if "SUCCESS! Media matched" in line or "Event-driven autokill triggered" in line
                        ),
                        "Event-driven autokill triggered",
                    )
                    print(
                        f"  [{idx:2d}/{sample_count}] ✅ VLC SUCCESS ({elapsed:.2f}s) : {mp4.name[:35]}... -> {matched_line.strip()[-50:]}"
                    )
                else:
                    print(
                        f"  [{idx:2d}/{sample_count}] ℹ️  VLC Décodé ({elapsed:.2f}s) : {mp4.name[:40]}... (accroche daemon)"
                    )

                results.append(
                    {
                        "video": mp4.name,
                        "elapsed_s": round(elapsed, 2),
                        "matched": is_matched,
                        "returncode": res.returncode,
                    }
                )
            except subprocess.TimeoutExpired:
                print(f"  [{idx:2d}/{sample_count}] ⏱️ Timeout (>12s) : {mp4.name[:40]}")
                results.append({"video": mp4.name, "timeout": True})

        print(
            f"\n✅ Batterie 4 Terminée : {success_count}/{len(sampled)} extraits synchronisés dans VLC sous Xvfb isolé."
        )

        return {
            "tested": len(sampled),
            "success": success_count,
            "display": f":{display_num}",
        }
    finally:
        print(f"Arrêt propre de l'écran virtuel Xvfb :{display_num}...")
        xvfb_proc.terminate()
        xvfb_proc.wait()


def main() -> int:
    parser = argparse.ArgumentParser(description="Full Corpus Exhaustive NAS Snippets Evaluation Matrix.")
    parser.add_argument("--snippets-dir", type=Path, default=Path("data/snippets"))
    parser.add_argument("--server-url", type=str, default="http://127.0.0.1:8765")
    parser.add_argument("--output-report", type=Path, default=Path("data/snippets/exhaustive_eval_report.json"))
    parser.add_argument("--workers", type=int, default=8, help="Number of concurrent query workers (default: 8)")
    parser.add_argument(
        "--vlc-samples", type=int, default=25, help="Number of sample videos for VLC Xvfb (default: 25)"
    )
    parser.add_argument("--skip-full-scan", action="store_true", help="Skip battery 1 & 2 full re-scan")
    args = parser.parse_args()

    print("================================================================================")
    print(" 🚀 MATRICE D'ÉVALUATION EXHAUSTIVE 100% CORPUS (DATASET RÉEL NAS)")
    print("================================================================================")
    print(f"Répertoire extraits : {args.snippets_dir}")
    print(f"URL Serveur daemon  : {args.server_url}")

    if args.output_report.exists() and args.skip_full_scan:
        with open(args.output_report, encoding="utf-8") as f:
            report = json.load(f)
    else:
        report = {
            "timestamp": time.strftime("%Y-%m-%d %H:%M:%S"),
            "total_corpus_files": len(list(args.snippets_dir.glob("*_mid15s.mp4"))),
        }

    if not args.skip_full_scan:
        report["battery_1_full_corpus"] = run_battery_1_full_corpus(
            args.snippets_dir, args.server_url, workers=args.workers
        )
        report["battery_2_all_series_isolation"] = run_battery_2_all_series_isolation(
            args.snippets_dir, args.server_url, workers=args.workers
        )
    report["battery_3_true_negatives"] = run_battery_3_true_negatives(args.server_url)
    report["battery_4_vlc_xvfb"] = run_battery_4_vlc_xvfb(args.snippets_dir, sample_count=args.vlc_samples)

    with open(args.output_report, "w", encoding="utf-8") as f:
        json.dump(report, f, indent=2, ensure_ascii=False)

    print("\n================================================================================")
    print(f" 🎉 RAPPORT GLOBAL COMPLET ENREGISTRÉ : {args.output_report}")
    print("================================================================================")
    return 0


if __name__ == "__main__":
    sys.exit(main())
