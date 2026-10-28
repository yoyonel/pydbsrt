#!/usr/bin/env python3
"""Simultaneous side-by-side telemetry comparison: Python vs Rust."""

import json
import subprocess
import sys
from pathlib import Path

# Add project root to sys.path
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from scripts.dbsrt_top import scan_active_jobs


def run_comparison():
    print("=" * 80)
    print("🔬 COMPARATIF TÉLÉMÉTRIE TEMPS RÉEL : PYTHON (scripts/dbsrt_top.py) vs RUST (dbsrt top)")
    print("=" * 80)

    # 1. Run Python sampler
    py_jobs = scan_active_jobs()

    # 2. Run Rust probe sampler
    try:
        proc = subprocess.run(
            ["./target/release/dbsrt", "top", "--json", "--refresh-ms", "500"],
            capture_output=True,
            text=True,
            check=True,
        )
        rs_data = json.loads(proc.stdout)
    except Exception as e:
        print(f"❌ Erreur lors de l'appel à Rust dbsrt top --json: {e}")
        return

    rs_jobs = rs_data.get("active_jobs", [])

    print("\n📊 Nombre de Workers Détectés :")
    print(f"   • Python : {len(py_jobs)} worker(s)")
    print(f"   • Rust   : {len(rs_jobs)} worker(s)")

    print("\n🌐 Débit Réseau / CIFS Global :")
    py_total_rate = sum(j.current_rate_mib_s for j in py_jobs)
    rs_vfs_rate = rs_data.get("total_rate_mib_s", 0.0)
    rs_net_rx = rs_data.get("net_rx_mib_s", 0.0)
    print(f"   • Python (VFS rchar)   : {py_total_rate:.1f} MB/s")
    print(f"   • Rust   (VFS rchar)   : {rs_vfs_rate:.1f} MB/s")
    print(f"   • Rust   (Physique RX) : {rs_net_rx:.1f} MB/s")

    print("\n💻 Charge Système Hôte :")
    rs_cpu = rs_data.get("host_cpu_pct", 0.0)
    rs_ram_used = rs_data.get("ram_used_gib", 0.0)
    rs_ram_total = rs_data.get("ram_total_gib", 0.0)
    print(f"   • Rust Host CPU : {rs_cpu:.1f}%")
    print(f"   • Rust Host RAM : {rs_ram_used:.1f} / {rs_ram_total:.1f} GiB")

    # Match by PID
    py_by_pid = {j.pid: j for j in py_jobs}
    rs_by_pid = {j["pid"]: j for j in rs_jobs}
    all_pids = sorted(set(py_by_pid.keys()) | set(rs_by_pid.keys()))

    if not all_pids:
        print("\nℹ️  Aucune tâche active (ffmpeg / dbsrt) en cours d'exécution.")
        return

    print("\n🔍 Comparaison Détaillée par Processus (PID) :")
    header = f"{'PID':<8} {'MOTEUR (Py/Rs)':<25} {'PROGRESSION':<20} {'DÉBIT VFS':<20} {'CPU %':<15} {'RAM MB':<15}"
    print("-" * len(header))
    print(header)
    print("-" * len(header))

    for pid in all_pids:
        py_j = py_by_pid.get(pid)
        rs_j = rs_by_pid.get(pid)

        py_eng = py_j.engine if py_j else "N/A"
        rs_eng = rs_j.get("engine", "N/A") if rs_j else "N/A"
        eng_str = f"{py_eng[:11]} / {rs_eng[:11]}"

        py_prog = f"{py_j.progress_ratio * 100:.1f}%" if py_j else "N/A"
        rs_prog = f"{rs_j.get('progress_ratio', 0.0) * 100:.1f}%" if rs_j else "N/A"
        prog_str = f"{py_prog} / {rs_prog}"

        py_rate = f"{py_j.current_rate_mib_s:.1f} MB/s" if py_j else "N/A"
        rs_rate = f"{rs_j.get('current_rate_mib_s', 0.0):.1f} MB/s" if rs_j else "N/A"
        rate_str = f"{py_rate} / {rs_rate}"

        py_cpu = f"{py_j.cpu_percent:.1f}%" if py_j else "N/A"
        rs_cpu = f"{rs_j.get('cpu_percent', 0.0):.1f}%" if rs_j else "N/A"
        cpu_str = f"{py_cpu} / {rs_cpu}"

        py_ram = f"{py_j.ram_rss_mib:.1f} MB" if py_j else "N/A"
        rs_ram = f"{rs_j.get('ram_rss_mib', 0.0):.1f} MB" if rs_j else "N/A"
        ram_str = f"{py_ram} / {rs_ram}"

        print(f"{pid:<8} {eng_str:<25} {prog_str:<20} {rate_str:<20} {cpu_str:<15} {ram_str:<15}")

    print("=" * 80)


if __name__ == "__main__":
    run_comparison()
