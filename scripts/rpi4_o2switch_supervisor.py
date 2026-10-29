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
import os
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import Dict

# Named Operational Constants
DEFAULT_CHECK_INTERVAL_SECS: int = 60
DEFAULT_PULL_INTERVAL_SECS: int = 600
DEFAULT_SSH_TIMEOUT_SECS: int = 15
DEFAULT_DAY_WORKERS: int = 2
DEFAULT_NIGHT_WORKERS: int = 4
NIGHT_START_HOUR: int = 23
NIGHT_END_HOUR: int = 7
DEFAULT_REMOTE_HOST: str = "o2switch"
DEFAULT_REMOTE_BASE: str = "dbsrt"
PID_FILE_PATH: str = "run/rpi4_supervisor.pid"
LOG_FILE_PATH: str = "logs/rpi4_supervisor.log"


def log(msg: str) -> None:
    """Logs timestamped message to stdout and append-only log file."""
    ts = datetime.datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    formatted = f"[{ts}] {msg}"
    print(formatted, flush=True)
    try:
        Path(LOG_FILE_PATH).parent.mkdir(parents=True, exist_ok=True)
        with open(LOG_FILE_PATH, "a", encoding="utf-8") as f:
            f.write(formatted + "\n")
    except Exception:
        pass


def compute_desired_workers() -> int:
    """Determines optimal concurrency based on wall-clock time window.

    Returns:
        4 workers during night window (23:00 - 07:00), 2 workers during daytime.
    """
    now_hour = datetime.datetime.now().hour
    if now_hour >= NIGHT_START_HOUR or now_hour < NIGHT_END_HOUR:
        return DEFAULT_NIGHT_WORKERS
    return DEFAULT_DAY_WORKERS


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
    """Main continuous watchdog loop."""
    log(f"🚀 RPi4 Supervisor Agent active (polling: {check_interval}s, pull: {pull_interval}s).")
    last_pull_time = time.time()

    while True:
        desired_workers = compute_desired_workers()
        status = check_remote_batch_status(remote_host, remote_base)

        if not status.get("running"):
            log("⚠️ Remote batch worker is NOT running. Triggering auto-healing restart...")
            restart_remote_batch(remote_host, remote_base, desired_workers)
        else:
            log(f"● Remote batch healthy (PID {status.get('pid')}, target workers: {desired_workers}).")

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

    # Write supervisor PID
    Path(PID_FILE_PATH).parent.mkdir(parents=True, exist_ok=True)
    with open(PID_FILE_PATH, "w", encoding="utf-8") as f:
        f.write(str(os.getpid()))

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
