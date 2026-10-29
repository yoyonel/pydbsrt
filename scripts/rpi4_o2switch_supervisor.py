#!/usr/bin/env python3
"""RPi4 Autonomous Watchdog Supervisor for o2switch Remote Batch Ingestion.

Runs as a 24/7 systemd user service on Raspberry Pi 4 (or local host) to:
1. Probe the remote batch worker lifecycle on o2switch (via o2switch_ctl.sh batch-status).
2. Restart the batch worker only when it crashed or never started (never after a manual
   stop or a drained queue), resuming the same batch type.
3. Autoscale worker concurrency (2-6) from the measured CloudLinux LVE CPU load.
4. Periodically pull finished .phash files, subtitles and reports into the local data/ tree.
"""

from __future__ import annotations

import argparse
import datetime
import fcntl
import os
import signal
import subprocess
import sys
import time
from pathlib import Path
from typing import Dict, List, Tuple

# Named Operational Constants
DEFAULT_CHECK_INTERVAL_SECS: int = 60
DEFAULT_PULL_INTERVAL_SECS: int = 600
DEFAULT_SSH_TIMEOUT_SECS: int = 15
DEFAULT_REMOTE_HOST: str = "o2switch"
DEFAULT_REMOTE_BASE: str = "dbsrt"
PID_FILE_PATH: str = "run/rpi4_supervisor.pid"
CTL_SCRIPT: str = "./scripts/o2switch_ctl.sh"
# batch-status states that call for an automatic restart of the same batch type
RESTARTABLE_STATES: Tuple[str, ...] = ("crashed", "absent")
# A drained queue hands the CPUs over to the other library, so o2switch never idles
NEXT_BATCH_TYPE: Dict[str, str] = {"tvshow": "movies", "movies": "tvshow"}
# Minimum delay between two launches, so a queue of always-failing files cannot relaunch in a tight loop
RELAUNCH_BACKOFF_SECS: int = 1800

# Autoscaling Engine Constants (CloudLinux LVE on o2switch has 12 vCPUs allocated = 1200% max)
DEFAULT_LVE_TOTAL_VCPUS: float = 12.0
AUTOSCALE_MIN_WORKERS: int = 2
# Matches the worker CPU budget: 9 vCPU // 2 FFmpeg threads per video
AUTOSCALE_MAX_WORKERS: int = 4
AUTOSCALE_DEFAULT_WORKERS: int = 4
AUTOSCALE_TARGET_LOW_PCT: float = 78.0  # Scale up concurrency if rolling average LVE CPU < 78%
AUTOSCALE_TARGET_HIGH_PCT: float = 92.0  # Scale down concurrency if rolling average LVE CPU > 92%
AUTOSCALE_EMERGENCY_SPIKE_PCT: float = 96.0  # Instant emergency throttle if single tick exceeds 96%
AUTOSCALE_COOLDOWN_SECS: int = 180  # Cooldown between scale adjustments (3 min to observe steady state)
AUTOSCALE_SAMPLE_WINDOW: int = 3  # Number of 60s probe ticks in rolling average window

# Prints clock ticks per second, then /proc/<pid>/stat of every process owned by the ssh user
CPU_TICKS_PROBE_CMD: str = (
    'getconf CLK_TCK; for d in /proc/[0-9]*; do [ -O "$d" ] && cat "$d/stat" 2>/dev/null; done; true'
)

CpuSample = Tuple[float, Dict[int, float]]  # (monotonic time, cumulative CPU seconds per pid)


def log(msg: str) -> None:
    """Logs timestamped message to stdout."""
    ts = datetime.datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    formatted = f"[{ts}] {msg}"
    print(formatted, flush=True)


def _ctl_env(remote_host: str, remote_base: str) -> Dict[str, str]:
    env = os.environ.copy()
    env["O2SWITCH_HOST"] = remote_host
    env["REMOTE_BASE"] = remote_base
    return env


def parse_proc_cpu_seconds(output: str) -> Dict[int, float]:
    """Parses CPU_TICKS_PROBE_CMD output into cumulative user+system CPU seconds per pid."""
    lines = output.splitlines()
    clk_tck = float(lines[0])
    cpu_secs: Dict[int, float] = {}
    for line in lines[1:]:
        r_idx = line.rfind(")")
        if r_idx == -1:
            continue
        # After "comm)": fields[11]=utime, fields[12]=stime
        fields = line[r_idx + 2 :].split()
        try:
            cpu_secs[int(line.split(" ", 1)[0])] = (int(fields[11]) + int(fields[12])) / clk_tck
        except (ValueError, IndexError):
            continue
    return cpu_secs


def compute_lve_cpu_pct(prev: CpuSample, cur: CpuSample, total_vcpus: float = DEFAULT_LVE_TOTAL_VCPUS) -> float:
    """CPU usage over the interval between two samples, as a percentage of the LVE allocation.

    A pid absent from `prev` started during the interval, so all its CPU time counts.
    Processes that exited during the interval are missed (slight under-estimate).
    """
    elapsed = cur[0] - prev[0]
    if elapsed <= 0:
        return 0.0
    used = sum(max(0.0, secs - prev[1].get(pid, 0.0)) for pid, secs in cur[1].items())
    return used / elapsed / total_vcpus * 100.0


def sample_remote_cpu(remote_host: str) -> CpuSample | None:
    """Samples cumulative CPU seconds of all remote processes of the ssh user."""
    cmd = ["ssh", "-o", "BatchMode=yes", remote_host, CPU_TICKS_PROBE_CMD]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=DEFAULT_SSH_TIMEOUT_SECS)
        if res.returncode == 0 and res.stdout.strip():
            return time.monotonic(), parse_proc_cpu_seconds(res.stdout)
    except Exception as err:
        log(f"⚠️ Failed probing CPU on {remote_host}: {err}")
    return None


def get_remote_desired_workers(
    remote_host: str, remote_base: str, default_workers: int = AUTOSCALE_DEFAULT_WORKERS
) -> int:
    """Reads active desired concurrency from remote run/desired_workers control file."""
    cmd = ["ssh", "-o", "BatchMode=yes", remote_host, f"cat {remote_base}/run/desired_workers 2>/dev/null || true"]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=DEFAULT_SSH_TIMEOUT_SECS)
        if res.returncode == 0 and res.stdout.strip().isdigit():
            return int(res.stdout.strip())
    except Exception:
        pass
    return default_workers


def get_remote_running_jobs(remote_host: str) -> int | None:
    """Counts in-flight dbsrt media:add jobs on the remote host (None when the probe fails)."""
    # The bracket keeps pgrep from matching the ssh shell whose command line holds the pattern
    cmd = ["ssh", "-o", "BatchMode=yes", remote_host, "pgrep -cf '[d]bsrt media:add' || true"]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=DEFAULT_SSH_TIMEOUT_SECS)
        if res.returncode == 0 and res.stdout.strip().isdigit():
            return int(res.stdout.strip())
    except Exception as err:
        log(f"⚠️ Failed counting running jobs on {remote_host}: {err}")
    return None


def set_remote_desired_workers(remote_host: str, remote_base: str, count: int) -> bool:
    """Updates remote desired concurrency through o2switch_ctl.sh batch-workers (validated bounds)."""
    cmd = [CTL_SCRIPT, "batch-workers", str(count)]
    try:
        res = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=DEFAULT_SSH_TIMEOUT_SECS,
            env=_ctl_env(remote_host, remote_base),
        )
        if res.returncode != 0:
            log(f"❌ Failed setting remote workers: {res.stderr or res.stdout}")
        return res.returncode == 0
    except Exception as err:
        log(f"❌ Failed setting remote workers: {err}")
        return False


def check_remote_batch_status(remote_host: str, remote_base: str) -> Dict[str, object]:
    """Queries remote batch lifecycle via o2switch_ctl.sh batch-status.

    Returns:
        Dict with keys 'state' (running|stopped|finished|crashed|absent|unknown), 'pid' (int | None)
        and 'type' (tvshow|movies). 'unknown' means the probe itself failed.
    """
    unknown: Dict[str, object] = {"state": "unknown", "pid": None, "type": "tvshow"}
    try:
        res = subprocess.run(
            [CTL_SCRIPT, "batch-status"],
            capture_output=True,
            text=True,
            timeout=DEFAULT_SSH_TIMEOUT_SECS,
            env=_ctl_env(remote_host, remote_base),
        )
    except Exception as err:
        log(f"⚠️ Failed querying status on {remote_host}: {err}")
        return unknown
    if res.returncode != 0:
        return unknown
    for line in res.stdout.splitlines():
        if line.startswith("BATCH_STATE="):
            fields = dict(part.split("=", 1) for part in line.split() if "=" in part)
            pid = fields.get("PID", "")
            return {
                "state": fields.get("BATCH_STATE", "unknown"),
                "pid": int(pid) if pid.isdigit() else None,
                "type": fields.get("TYPE", "tvshow"),
            }
    return unknown


def restart_remote_batch(remote_host: str, remote_base: str, workers: int, batch_type: str = "tvshow") -> bool:
    """Restarts remote batch ingestion of the given type with specified concurrency."""
    log(f"🔄 Launching remote {batch_type} batch on {remote_host} (concurrency: {workers} workers)...")
    cmd = [CTL_SCRIPT, f"batch-{batch_type}", "--workers", str(workers), "--fresh-only", "--daemon"]
    try:
        res = subprocess.run(cmd, capture_output=True, text=True, timeout=120, env=_ctl_env(remote_host, remote_base))
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
    try:
        res = subprocess.run(
            [CTL_SCRIPT, "pull-catalog"],
            capture_output=True,
            text=True,
            timeout=600,
            env=_ctl_env(remote_host, remote_base),
        )
        if res.returncode == 0:
            log("✓ Catalog synchronization complete.")
            return True
        log(f"⚠️ Catalog pull returned warning: {res.stderr or res.stdout}")
        return False
    except Exception as err:
        log(f"⚠️ Catalog pull failed: {err}")
        return False


def decide_next_batch(state: str, batch_type: str, secs_since_last_launch: float) -> str | None:
    """Returns the batch type to launch now, or None.

    Crashed/absent batches resume their type; a finished batch switches to the other library.
    Manual stops, aborted batches and failed probes never trigger a launch.
    """
    if state in RESTARTABLE_STATES:
        next_type = batch_type
    elif state == "finished":
        next_type = NEXT_BATCH_TYPE.get(batch_type, "tvshow")
    else:
        return None
    return next_type if secs_since_last_launch >= RELAUNCH_BACKOFF_SECS else None


def decide_workers(
    cpu: float, cpu_history: List[float], cur_workers: int, cooldown_elapsed: bool, running_jobs: int | None = None
) -> int:
    """Returns the new worker count for one CPU sample (cur_workers when unchanged).

    `cpu_history` is the rolling window, already including `cpu`. Scale-downs wait while
    `running_jobs` still exceeds `cur_workers`: in-flight videos are never stopped, so a lower
    target only bites when they finish and lowering it again would starve the next wave.
    """
    can_scale_down = cur_workers > AUTOSCALE_MIN_WORKERS and (running_jobs is None or running_jobs <= cur_workers)
    if cpu >= AUTOSCALE_EMERGENCY_SPIKE_PCT and can_scale_down:
        return cur_workers - 1
    if len(cpu_history) < AUTOSCALE_SAMPLE_WINDOW or not cooldown_elapsed:
        return cur_workers
    avg_cpu = sum(cpu_history) / len(cpu_history)
    if avg_cpu < AUTOSCALE_TARGET_LOW_PCT and cur_workers < AUTOSCALE_MAX_WORKERS:
        return cur_workers + 1
    if avg_cpu > AUTOSCALE_TARGET_HIGH_PCT and can_scale_down:
        return cur_workers - 1
    return cur_workers


def supervisor_loop(remote_host: str, remote_base: str, check_interval: int, pull_interval: int) -> None:
    """Main continuous watchdog and adaptive autoscaling loop."""
    log(
        f"🚀 RPi4 Supervisor Agent active (polling: {check_interval}s, pull: {pull_interval}s, "
        f"autoscaling: {AUTOSCALE_MIN_WORKERS}-{AUTOSCALE_MAX_WORKERS} workers, target: {AUTOSCALE_TARGET_LOW_PCT:.0f}-{AUTOSCALE_TARGET_HIGH_PCT:.0f}% LVE CPU)."
    )
    last_pull_time = time.time()
    last_scale_time = 0.0
    last_launch_time = 0.0
    cpu_history: List[float] = []
    prev_sample: CpuSample | None = None

    while True:
        status = check_remote_batch_status(remote_host, remote_base)
        state = str(status["state"])

        if state == "running":
            cur_workers = get_remote_desired_workers(remote_host, remote_base)
            sample = sample_remote_cpu(remote_host)
            if sample is not None and prev_sample is not None:
                cpu = compute_lve_cpu_pct(prev_sample, sample)
                cpu_history = (cpu_history + [cpu])[-AUTOSCALE_SAMPLE_WINDOW:]
                now = time.time()
                running_jobs = get_remote_running_jobs(remote_host)
                new_workers = decide_workers(
                    cpu, cpu_history, cur_workers, now - last_scale_time >= AUTOSCALE_COOLDOWN_SECS, running_jobs
                )
                avg_cpu = sum(cpu_history) / len(cpu_history)
                if new_workers != cur_workers and set_remote_desired_workers(remote_host, remote_base, new_workers):
                    direction = "📈 [AUTOSCALE UP]" if new_workers > cur_workers else "📉 [AUTOSCALE DOWN]"
                    log(f"{direction} CPU {cpu:.1f}% (avg {avg_cpu:.1f}%): {cur_workers} -> {new_workers} workers.")
                    last_scale_time = now
                    cpu_history.clear()
                else:
                    log(
                        f"● Remote batch healthy (PID {status['pid']}, CPU: {cpu:.1f}% [avg {avg_cpu:.1f}%, "
                        f"{len(cpu_history)}/{AUTOSCALE_SAMPLE_WINDOW} samples], workers: {cur_workers}, running: {running_jobs})."
                    )
            else:
                log(f"● Remote batch healthy (PID {status['pid']}, workers: {cur_workers}, CPU baseline pending).")
            prev_sample = sample
        elif state == "unknown":
            log("⚠️ Remote batch status probe failed; no action this cycle.")
        else:
            next_type = decide_next_batch(state, str(status["type"]), time.time() - last_launch_time)
            if next_type is None:
                log(f"○ Remote batch is {state} ({status['type']}); no launch this cycle.")
            else:
                cur_workers = get_remote_desired_workers(remote_host, remote_base)
                log(
                    f"⚠️ Remote batch is {state} ({status['type']}). Launching {next_type} with {cur_workers} workers..."
                )
                restart_remote_batch(remote_host, remote_base, cur_workers, next_type)
                cpu_history.clear()
                prev_sample = None
                last_scale_time = last_launch_time = time.time()

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

    # Exclusive single-instance process lock, released by the kernel when the process exits.
    # The file is never unlinked: a new inode would let a second instance lock it concurrently.
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
        sys.exit(0)

    signal.signal(signal.SIGINT, _sig_handler)
    signal.signal(signal.SIGTERM, _sig_handler)

    supervisor_loop(
        remote_host=args.remote_host,
        remote_base=args.remote_base,
        check_interval=args.check_interval,
        pull_interval=args.pull_interval,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
