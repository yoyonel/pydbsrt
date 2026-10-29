#!/usr/bin/env python3
"""RPi4 Autonomous Watchdog Supervisor for o2switch Remote Batch Ingestion.

Runs as a 24/7 background agent on Raspberry Pi 4 (or local host) to:
1. Probe the remote batch worker on o2switch (PID & screen session).
2. Automatically heal and restart the batch worker if dead/killed.
3. Automatically sync finished .phash files to local PostgreSQL / Zero-DB.
4. Adapt concurrency (4 workers night, 2 workers day) to preserve NAS I/O.
"""

from __future__ import annotations

import argparse
import datetime
import fcntl
import os
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Dict, List

# Named Operational Constants
DEFAULT_CHECK_INTERVAL_SECS: int = 60
DEFAULT_PULL_INTERVAL_SECS: int = 600
DEFAULT_SSH_TIMEOUT_SECS: int = 15
DEFAULT_REMOTE_HOST: str = "o2switch"
DEFAULT_REMOTE_BASE: str = "dbsrt"
PID_FILE_PATH: str = "run/rpi4_supervisor.pid"
LOG_FILE_PATH: str = "logs/rpi4_supervisor.log"

# Autoscaling Engine Constants (CloudLinux LVE on o2switch has 12 vCPUs allocated = 1200% max)
DEFAULT_LVE_TOTAL_VCPUS: float = 12.0
AUTOSCALE_MIN_WORKERS: int = 2
AUTOSCALE_MAX_WORKERS: int = 6
AUTOSCALE_DEFAULT_WORKERS: int = 4
AUTOSCALE_TARGET_LOW_PCT: float = 78.0  # Scale up concurrency if rolling average LVE CPU < 78%
AUTOSCALE_TARGET_HIGH_PCT: float = 92.0  # Scale down concurrency if rolling average LVE CPU > 92%
AUTOSCALE_EMERGENCY_SPIKE_PCT: float = 96.0  # Instant emergency throttle if single tick exceeds 96%
AUTOSCALE_COOLDOWN_SECS: int = 180  # Cooldown between scale adjustments (3 min to observe steady state)
AUTOSCALE_SAMPLE_WINDOW: int = 3  # Number of 60s probe ticks in rolling average window


def log(msg: str) -> None:
    """Logs timestamped message to stdout."""
    ts = datetime.datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    formatted = f"[{ts}] {msg}"
    print(formatted, flush=True)


def get_remote_lve_cpu_pct(remote_host: str) -> float | None:
    """Measures current remote CloudLinux LVE CPU usage percentage (12 vCPUs allocation)."""
    cmd = ["ssh", remote_host, "ps -u $(whoami) -o %cpu | awk '{s+=$1} END {print s}'"]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=DEFAULT_SSH_TIMEOUT_SECS)
        if res.returncode == 0 and res.stdout.strip():
            total_cpu = float(res.stdout.strip())
            return total_cpu / DEFAULT_LVE_TOTAL_VCPUS
    except Exception as err:
        log(f"⚠️ Failed probing CPU on {remote_host}: {err}")
    return None


def get_remote_desired_workers(
    remote_host: str, remote_base: str, default_workers: int = AUTOSCALE_DEFAULT_WORKERS
) -> int:
    """Reads active desired concurrency from remote run/desired_workers control file."""
    cmd = ["ssh", remote_host, f"cat {remote_base}/run/desired_workers 2>/dev/null || true"]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=DEFAULT_SSH_TIMEOUT_SECS)
        if res.returncode == 0 and res.stdout.strip().isdigit():
            return int(res.stdout.strip())
    except Exception:
        pass
    return default_workers


def set_remote_desired_workers(remote_host: str, remote_base: str, count: int) -> bool:
    """Updates remote desired concurrency control file."""
    cmd = [
        "ssh",
        remote_host,
        f"mkdir -p {remote_base}/run && echo {count} > {remote_base}/run/desired_workers",
    ]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=DEFAULT_SSH_TIMEOUT_SECS)
        return res.returncode == 0
    except Exception as err:
        log(f"❌ Failed setting remote workers: {err}")
        return False


def check_remote_batch_status(remote_host: str, remote_base: str) -> Dict[str, object]:
    """Queries remote batch worker process state via o2switch_ctl.sh.

    Returns:
        Dict containing keys: 'running' (bool), 'pid' (int | None), 'threads' (int).
    """
    cmd = ["./scripts/o2switch_ctl.sh", "batch-status"]
    env = os.environ.copy()
    env["O2SWITCH_HOST"] = remote_host
    env["REMOTE_BASE"] = remote_base
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=DEFAULT_SSH_TIMEOUT_SECS, env=env)
        is_running = "Batch worker is RUNNING" in res.stdout
        pid: int | None = None
        for line in res.stdout.splitlines():
            if "PID:" in line:
                parts = line.split("PID:")
                if len(parts) > 1:
                    try:
                        pid = int(parts[1].strip().split()[0].replace(")", ""))
                    except ValueError:
                        pass
        return {"running": is_running, "pid": pid, "output": res.stdout}
    except Exception as err:
        log(f"⚠️ Failed querying status on {remote_host}: {err}")
        return {"running": False, "pid": None, "error": str(err)}


def restart_remote_batch(remote_host: str, remote_base: str, workers: int) -> bool:
    """Restarts remote batch ingestion daemon on o2switch via o2switch_ctl.sh."""
    log(f"🔄 Launching remote batch on {remote_host} (concurrency: {workers} workers)...")
    cmd = [
        "./scripts/o2switch_ctl.sh",
        "batch-tvshow",
        "--workers",
        str(workers),
        "--fresh-only",
        "--daemon",
    ]
    env = os.environ.copy()
    env["O2SWITCH_HOST"] = remote_host
    env["REMOTE_BASE"] = remote_base
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=30, env=env)
        success = res.returncode == 0 and "Started persistent background batch worker" in res.stdout
        if success:
            log(f"✓ Remote batch successfully started with {workers} workers.")
        else:
            log(f"❌ Failed starting batch: {res.stderr or res.stdout}")
        return success
    except Exception as err:
        log(f"❌ Exception starting remote batch: {err}")
        return False


def pull_remote_catalog(remote_host: str, remote_base: str) -> bool:
    """Pulls newly computed pHash and subtitle artifacts to local storage."""
    log("📥 Synchronizing catalog and .phash artifacts from o2switch...")
    cmd = ["./scripts/o2switch_ctl.sh", "pull-catalog"]
    env = os.environ.copy()
    env["O2SWITCH_HOST"] = remote_host
    env["REMOTE_BASE"] = remote_base
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=120, env=env)
        if res.returncode == 0:
            log("✓ Catalog synchronization complete.")
            return True
        log(f"⚠️ Catalog pull returned warning: {res.stderr or res.stdout}")
        return False
    except Exception as err:
        log(f"⚠️ Catalog pull failed: {err}")
        return False


def supervisor_loop(
    remote_host: str,
    remote_base: str,
    check_interval: int,
    pull_interval: int,
    stop_event: threading.Event | None = None,
) -> None:
    """Main continuous watchdog and adaptive autoscaling loop."""
    log(
        f"🚀 RPi4 Supervisor Agent active (polling: {check_interval}s, pull: {pull_interval}s, "
        f"autoscaling: {AUTOSCALE_MIN_WORKERS}-{AUTOSCALE_MAX_WORKERS} workers, target: {AUTOSCALE_TARGET_LOW_PCT:.0f}-{AUTOSCALE_TARGET_HIGH_PCT:.0f}% LVE CPU)."
    )
    last_pull_time = time.time()
    last_scale_time = 0.0
    cpu_history: List[float] = []

    while True:
        status = check_remote_batch_status(remote_host, remote_base)
        cur_workers = get_remote_desired_workers(remote_host, remote_base)

        if not status.get("running"):
            log(f"⚠️ Remote batch worker is NOT running. Triggering auto-healing restart with {cur_workers} workers...")
            restart_remote_batch(remote_host, remote_base, cur_workers)
            cpu_history.clear()
            last_scale_time = time.time()
        else:
            cpu = get_remote_lve_cpu_pct(remote_host)
            if cpu is not None:
                # Emergency instant spike throttle (safety guard)
                if cpu >= AUTOSCALE_EMERGENCY_SPIKE_PCT and cur_workers > AUTOSCALE_MIN_WORKERS:
                    new_workers = cur_workers - 1
                    set_remote_desired_workers(remote_host, remote_base, new_workers)
                    log(
                        f"🚨 [EMERGENCY THROTTLE] Instant CPU spike {cpu:.1f}% >= {AUTOSCALE_EMERGENCY_SPIKE_PCT}%. "
                        f"Throttled {cur_workers} -> {new_workers} workers immediately."
                    )
                    last_scale_time = time.time()
                    cpu_history.clear()
                else:
                    cpu_history.append(cpu)
                    if len(cpu_history) > AUTOSCALE_SAMPLE_WINDOW:
                        cpu_history.pop(0)

                    avg_cpu = sum(cpu_history) / len(cpu_history)
                    now = time.time()

                    if len(cpu_history) >= AUTOSCALE_SAMPLE_WINDOW and (
                        now - last_scale_time >= AUTOSCALE_COOLDOWN_SECS
                    ):
                        if avg_cpu < AUTOSCALE_TARGET_LOW_PCT and cur_workers < AUTOSCALE_MAX_WORKERS:
                            new_workers = cur_workers + 1
                            set_remote_desired_workers(remote_host, remote_base, new_workers)
                            log(
                                f"📈 [AUTOSCALE UP] Remote CPU average is {avg_cpu:.1f}% (< {AUTOSCALE_TARGET_LOW_PCT}%). "
                                f"Scaling concurrency: {cur_workers} -> {new_workers} workers."
                            )
                            last_scale_time = now
                            cpu_history.clear()
                        elif avg_cpu > AUTOSCALE_TARGET_HIGH_PCT and cur_workers > AUTOSCALE_MIN_WORKERS:
                            new_workers = cur_workers - 1
                            set_remote_desired_workers(remote_host, remote_base, new_workers)
                            log(
                                f"📉 [AUTOSCALE DOWN] Remote CPU average is {avg_cpu:.1f}% (> {AUTOSCALE_TARGET_HIGH_PCT}%). "
                                f"Throttling concurrency: {cur_workers} -> {new_workers} workers."
                            )
                            last_scale_time = now
                            cpu_history.clear()
                        else:
                            log(
                                f"● Remote batch healthy (PID {status.get('pid')}, CPU: {cpu:.1f}% [avg {avg_cpu:.1f}%], "
                                f"workers: {cur_workers}, in target zone {AUTOSCALE_TARGET_LOW_PCT:.0f}-{AUTOSCALE_TARGET_HIGH_PCT:.0f}%)."
                            )
                    else:
                        samples_str = (
                            f" ({len(cpu_history)}/{AUTOSCALE_SAMPLE_WINDOW} samples)"
                            if len(cpu_history) < AUTOSCALE_SAMPLE_WINDOW
                            else ""
                        )
                        log(
                            f"● Remote batch healthy (PID {status.get('pid')}, CPU: {cpu:.1f}%, workers: {cur_workers}){samples_str}."
                        )
            else:
                log(f"● Remote batch healthy (PID {status.get('pid')}, target workers: {cur_workers}).")

        # Periodic catalog pull
        now = time.time()
        if now - last_pull_time >= pull_interval:
            pull_remote_catalog(remote_host, remote_base)
            last_pull_time = now

        time.sleep(check_interval)


def main() -> int:
    """CLI Entry point for RPi4 supervisor."""
    parser = argparse.ArgumentParser(description="RPi4 Autonomous Supervisor for o2switch Batch Ingestion")
    parser.add_argument("--remote-host", default=DEFAULT_REMOTE_HOST, help="Remote SSH host alias")
    parser.add_argument("--remote-base", default=DEFAULT_REMOTE_BASE, help="Remote project directory")
    parser.add_argument(
        "--check-interval", type=int, default=DEFAULT_CHECK_INTERVAL_SECS, help="Healthcheck polling interval in secs"
    )
    parser.add_argument(
        "--pull-interval", type=int, default=DEFAULT_PULL_INTERVAL_SECS, help="Catalog pull sync interval in secs"
    )
    args = parser.parse_args()

    # Exclusive single-instance process lock
    pid_path = Path(PID_FILE_PATH)
    pid_path.parent.mkdir(parents=True, exist_ok=True)
    lock_file = open(pid_path, "a+", encoding="utf-8")
    try:
        fcntl.flock(lock_file.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    except (BlockingIOError, OSError):
        log(f"⚠️ Another instance of supervisor is already running ({PID_FILE_PATH} locked). Exiting.")
        return 0

    lock_file.seek(0)
    lock_file.truncate()
    lock_file.write(str(os.getpid()))
    lock_file.flush()

    def _sig_handler(sig: int, _frame: object) -> None:
        log(f"🛑 Received signal {sig}. Stopping supervisor.")
        try:
            Path(PID_FILE_PATH).unlink(missing_ok=True)
        except Exception:
            pass
        sys.exit(0)

    signal.signal(signal.SIGINT, _sig_handler)
    signal.signal(signal.SIGTERM, _sig_handler)

    try:
        supervisor_loop(
            remote_host=args.remote_host,
            remote_base=args.remote_base,
            check_interval=args.check_interval,
            pull_interval=args.pull_interval,
        )
    finally:
        try:
            Path(PID_FILE_PATH).unlink(missing_ok=True)
        except Exception:
            pass
    return 0


if __name__ == "__main__":
    sys.exit(main())
