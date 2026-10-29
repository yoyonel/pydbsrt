#!/usr/bin/env python3
"""o2switch Remote Batch Processing Worker.

Orchestrates multi-worker parallel pHash extraction and subtitle ingestion on o2switch
shared hosting (CloudLinux LVE 12 vCPUs) by streaming directly from Synology NAS SFTP
via an in-memory HTTP bridge (Zero-NVMe / Zero local video disk copy).
"""

from __future__ import annotations

import argparse
import atexit
import json
import os
import re
import signal
import socketserver
import subprocess
import sys
import threading
import time
import traceback
import urllib.parse
from concurrent.futures import ThreadPoolExecutor, as_completed
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from typing import Dict, List, Set, Tuple


def _handle_signal(sig: int, _frame: object) -> None:
    sig_name = signal.Signals(sig).name
    msg = f"\n⚠️ [SIGNAL] Process received signal {sig_name} ({sig}) at {time.strftime('%Y-%m-%d %H:%M:%S')}\n"
    print(msg, flush=True)
    try:
        with open("logs/batch_worker.log", "a", encoding="utf-8") as f:
            f.write(msg)
    except Exception:
        pass
    sys.exit(128 + sig)


def _handle_uncaught_exception(exc_type: type, exc_value: BaseException, exc_tb: object) -> None:
    msg = "".join(traceback.format_exception(exc_type, exc_value, exc_tb))  # type: ignore
    err_str = f"\n❌ [CRASH] Uncaught Exception at {time.strftime('%Y-%m-%d %H:%M:%S')}:\n{msg}\n"
    print(err_str, flush=True)
    try:
        with open("logs/batch_worker.log", "a", encoding="utf-8") as f:
            f.write(err_str)
    except Exception:
        pass


@atexit.register
def _on_exit() -> None:
    msg = f"\nℹ️ [EXIT] Batch worker exiting at {time.strftime('%Y-%m-%d %H:%M:%S')}\n"
    print(msg, flush=True)


sys.excepthook = _handle_uncaught_exception
for _sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT, signal.SIGQUIT, signal.SIGUSR1, signal.SIGUSR2):
    try:
        signal.signal(_sig, _handle_signal)
    except Exception:
        pass


def _load_env_config(path: Path) -> Dict[str, str]:
    """Loads key-value pairs from an optional local unversioned environment file."""
    if not path.is_file():
        return {}
    config: Dict[str, str] = {}
    try:
        for line in path.read_text(encoding="utf-8").splitlines():
            stripped = line.strip()
            if not stripped or stripped.startswith("#"):
                continue
            if "=" in stripped:
                k, v = stripped.split("=", 1)
                config[k.strip()] = v.strip().strip("'\"")
    except Exception:
        pass
    return config


_CONFIG_ENV: Dict[str, str] = _load_env_config(Path("etc/.nas_config"))
if not _CONFIG_ENV:
    _CONFIG_ENV = _load_env_config(Path("etc/.env"))
if not _CONFIG_ENV:
    _CONFIG_ENV = _load_env_config(Path(".env"))

# Operational & System Constants
DEFAULT_WORKERS: int = 4
DEFAULT_BRIDGE_PORT: int = 8769
DEFAULT_NAS_HOST: str = os.getenv("NAS_HOST") or _CONFIG_ENV.get("NAS_HOST") or "nas.synology.me"
DEFAULT_NAS_USER: str = os.getenv("NAS_USER") or _CONFIG_ENV.get("NAS_USER") or "dbsrt-svc"
DEFAULT_NAS_BASE: str = "/tvshow"
DEFAULT_CHUNK_SIZE_BYTES: int = 128 * 1024
DEFAULT_CURL_TIMEOUT_SECS: int = 3600
DEFAULT_SPEED_LIMIT_BYTES_SEC: int = 1024
DEFAULT_SPEED_TIME_SECS: int = 60
DEFAULT_WORKER_TIMEOUT_SECS: int = 1200
DEFAULT_STALL_TIMEOUT_SECS: int = 60
DEFAULT_MAX_EXTRACTION_SECS: int = 1800
DEFAULT_COUPLINGS_PATH: str = "data/SRT/couplings.json"
DEFAULT_PHASH_DIR: str = "data/__PHASH__"
DEFAULT_SUBTITLES_DIR: str = "data/SRT"
DEFAULT_EVENTS_LOG_PATH: str = "data/reports/batch_tvshow_nightly.jsonl"
DEFAULT_BATCH_QUEUE_PATH: str = "run/batch_queue.json"
STREAM_ROUTE_PREFIX: str = "/stream/"
VIDEO_EXTENSIONS: Tuple[str, ...] = (".mkv", ".mp4", ".webm", ".avi")


class ThreadedHTTPServer(socketserver.ThreadingMixIn, HTTPServer):
    """Multi-threaded HTTP streaming bridge server."""

    daemon_threads = True
    allow_reuse_address = True


# Thread-safe tracker for actual physical bytes transferred per remote stream path
STREAM_BYTES_MAP: Dict[str, int] = {}
STREAM_BYTES_LOCK = threading.Lock()
FILE_SIZES_MAP: Dict[str, int] = {}


class StreamingBridgeHandler(BaseHTTPRequestHandler):
    """Pipes SFTP video streams directly into HTTP clients with zero disk buffering."""

    nas_host: str = DEFAULT_NAS_HOST
    nas_user: str = DEFAULT_NAS_USER
    nas_pass: str = ""

    def _get_content_type(self, path: str) -> str:
        lower = path.lower()
        if lower.endswith(".mkv"):
            return "video/x-matroska"
        if lower.endswith(".mp4"):
            return "video/mp4"
        if lower.endswith(".webm"):
            return "video/webm"
        return "video/octet-stream"

    def do_HEAD(self) -> None:
        """Handles HEAD requests for probing container metadata."""
        if not self.path.startswith(STREAM_ROUTE_PREFIX):
            self.send_error(404, "Not Found")
            return

        rel_path = self.path[len(STREAM_ROUTE_PREFIX) :]
        unquoted = urllib.parse.unquote(rel_path)
        if not unquoted.startswith("/"):
            unquoted = "/" + unquoted

        total_size = FILE_SIZES_MAP.get(unquoted)
        self.send_response(200)
        self.send_header("Content-Type", self._get_content_type(unquoted))
        self.send_header("Accept-Ranges", "bytes")
        if total_size is not None and total_size > 0:
            self.send_header("Content-Length", str(total_size))
        self.end_headers()

    def do_GET(self) -> None:
        """Handles GET request by piping SFTP stream from NAS via curl with Range support."""
        if not self.path.startswith(STREAM_ROUTE_PREFIX):
            self.send_error(404, "Not Found")
            return

        rel_path = self.path[len(STREAM_ROUTE_PREFIX) :]
        unquoted = urllib.parse.unquote(rel_path)
        # Ensure leading slash for SFTP URL path
        if not unquoted.startswith("/"):
            unquoted = "/" + unquoted

        # Build SFTP URL with separate -u flag to prevent special character corruption (% in password)
        path_enc = urllib.parse.quote(unquoted, safe="/")
        sftp_url = f"sftp://{self.nas_host}{path_enc}"

        range_header = self.headers.get("Range")
        range_spec = None
        start_byte = 0
        end_byte = None
        if range_header and range_header.startswith("bytes="):
            spec = range_header[6:].strip()
            if "-" in spec:
                p_start, p_end = spec.split("-", 1)
                if p_start:
                    try:
                        start_byte = int(p_start)
                    except ValueError:
                        pass
                if p_end:
                    try:
                        end_byte = int(p_end)
                    except ValueError:
                        pass
                range_spec = spec

        cmd = [
            "curl",
            "-s",
            "-k",
            "--tcp-nodelay",
            "-u",
            f"{self.nas_user}:{self.nas_pass}",
            "--retry",
            "2",
            "--speed-limit",
            str(DEFAULT_SPEED_LIMIT_BYTES_SEC),
            "--speed-time",
            str(DEFAULT_SPEED_TIME_SECS),
            "--max-time",
            str(DEFAULT_CURL_TIMEOUT_SECS),
        ]
        if range_spec:
            cmd.extend(["-r", range_spec])
        cmd.append(sftp_url)

        proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        first_chunk = b""
        try:
            first_chunk = proc.stdout.read(DEFAULT_CHUNK_SIZE_BYTES) if proc.stdout else b""
        except Exception:
            pass

        if not first_chunk:
            err_msg = ""
            if proc.stderr:
                try:
                    err_msg = proc.stderr.read().decode("utf-8", "ignore").strip()
                except Exception:
                    pass
            self.send_error(502, f"Bridge SFTP failed: {err_msg}")
            if proc.poll() is None:
                try:
                    proc.kill()
                    proc.wait(timeout=0.2)
                except Exception:
                    pass
            return

        if proc.stderr:
            try:
                proc.stderr.close()
            except Exception:
                pass

        total_size = FILE_SIZES_MAP.get(unquoted)
        content_type = self._get_content_type(unquoted)

        if range_spec:
            self.send_response(206)
            self.send_header("Content-Type", content_type)
            self.send_header("Accept-Ranges", "bytes")
            tot_str = str(total_size) if total_size else "*"
            end_str = str(end_byte) if end_byte is not None else "*"
            self.send_header("Content-Range", f"bytes {start_byte}-{end_str}/{tot_str}")
            if end_byte is not None and end_byte >= start_byte:
                self.send_header("Content-Length", str(end_byte - start_byte + 1))
        else:
            self.send_response(200)
            self.send_header("Content-Type", content_type)
            self.send_header("Accept-Ranges", "bytes")
            if total_size is not None and total_size > 0:
                self.send_header("Content-Length", str(total_size))
        self.end_headers()

        with STREAM_BYTES_LOCK:
            STREAM_BYTES_MAP[unquoted] = STREAM_BYTES_MAP.get(unquoted, 0) + len(first_chunk)
        try:
            self.wfile.write(first_chunk)
            while True:
                chunk = proc.stdout.read(DEFAULT_CHUNK_SIZE_BYTES) if proc.stdout else b""
                if not chunk:
                    break
                self.wfile.write(chunk)
                with STREAM_BYTES_LOCK:
                    STREAM_BYTES_MAP[unquoted] = STREAM_BYTES_MAP.get(unquoted, 0) + len(chunk)
        except (BrokenPipeError, ConnectionResetError):
            pass
        finally:
            if proc.stdout:
                try:
                    proc.stdout.close()
                except Exception:
                    pass
            if proc.stderr:
                try:
                    proc.stderr.close()
                except Exception:
                    pass
            if proc.poll() is None:
                try:
                    proc.terminate()
                    proc.wait(timeout=0.5)
                except Exception:
                    try:
                        proc.kill()
                        proc.wait(timeout=0.5)
                    except Exception:
                        pass
            else:
                try:
                    proc.wait(timeout=0.1)
                except Exception:
                    pass

    def log_message(self, format: str, *args: object) -> None:
        """Suppresses noisy per-chunk HTTP access logs."""
        return


def load_certified_stems(couplings_file: Path) -> Set[str]:
    """Loads all media stems already certified in couplings.json."""
    stems: Set[str] = set()
    if couplings_file.is_file():
        try:
            data = json.loads(couplings_file.read_text(encoding="utf-8"))
            for item in data.get("couplings", []):
                media_name = item.get("media_name", "")
                if media_name:
                    stems.add(media_name)
        except Exception as err:
            print(f"⚠️ Warning loading {couplings_file}: {err}", file=sys.stderr)
    return stems


def load_existing_phash_stems(phash_dir: Path) -> Set[str]:
    """Loads all media stems with existing .phash files on disk."""
    stems: Set[str] = set()
    if phash_dir.is_dir():
        for p in phash_dir.glob("*.phash"):
            try:
                if p.stat().st_size == 0:
                    continue
            except OSError:
                continue
            raw_name = urllib.parse.unquote(p.name)
            if raw_name.endswith(".phash"):
                raw_name = raw_name[:-6]
            raw_name = re.sub(r"\.\d+(\.\d+)?fps$", "", raw_name)
            for ext in VIDEO_EXTENSIONS:
                if raw_name.lower().endswith(ext):
                    raw_name = raw_name[: -len(ext)]
                    break
            stems.add(raw_name)
            stems.add(p.stem)
    return stems


def list_nas_videos_ftp(
    nas_host: str,
    nas_user: str,
    nas_pass: str,
    base_dir: str = "/tvshow",
    ftp_port: int = 21,
    max_depth: int = 8,
) -> List[Tuple[str, str, int]]:
    """Discovers video files on Synology NAS using standard FTP MLSD protocol (Zero curl/SFTP).

    Returns:
        List of tuples: (relative_path, filename, size_bytes).
    """
    import ftplib
    import posixpath
    import unicodedata

    clean_base = base_dir.lstrip("/")
    ftp = ftplib.FTP()
    ftp.connect(nas_host, ftp_port, timeout=15)
    ftp.login(nas_user, nas_pass)
    ftp.encoding = "utf-8"

    results: List[Tuple[str, str, int]] = []
    stack: List[str] = [clean_base]

    while stack:
        curr = stack.pop()
        try:
            entries = list(ftp.mlsd(curr))
        except Exception:
            continue
        for name, facts in entries:
            name = unicodedata.normalize("NFC", name)
            if name in (".", "..", "EMPTY_DIRS", "SRT", "@eaDir") or name.startswith("."):
                continue
            full_path = posixpath.join(curr, name)
            ftype = facts.get("type", "")
            if ftype == "dir":
                if full_path.count("/") <= max_depth:
                    stack.append(full_path)
            elif ftype == "file":
                if any(name.lower().endswith(ext) for ext in VIDEO_EXTENSIONS):
                    size = int(facts.get("size", 0))
                    rel = "/" + full_path if not full_path.startswith("/") else full_path
                    results.append((rel, name, size))

    try:
        ftp.quit()
    except Exception:
        pass
    return results


def list_nas_videos_recursive(
    nas_host: str,
    nas_user: str,
    nas_pass: str,
    base_dir: str,
    max_depth: int = 3,
) -> List[Tuple[str, str, int]]:
    """Recursively lists video files from Synology NAS via SFTP curl.

    Returns:
        List of tuples: (relative_path, filename, size_bytes).
    """
    results: List[Tuple[str, str, int]] = []
    dirs_to_visit: List[Tuple[str, int]] = [(base_dir, 0)]

    while dirs_to_visit:
        cur_dir, depth = dirs_to_visit.pop(0)
        dir_enc = urllib.parse.quote(cur_dir, safe="/")
        sftp_url = f"sftp://{nas_host}{dir_enc}/"
        cmd = ["curl", "-s", "-k", "-u", f"{nas_user}:{nas_pass}", sftp_url]
        res = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        if res.returncode != 0:
            continue

        for line in res.stdout.splitlines():
            line = line.strip()
            if not line or line.startswith("total "):
                continue
            parts = line.split(maxsplit=8)
            if len(parts) < 9:
                continue
            perms, size_str, name = parts[0], parts[4], parts[8]
            if name in {".", "..", "EMPTY_DIRS", "SRT"} or name.startswith("."):
                continue

            full_child = f"{cur_dir}/{name}".replace("//", "/")
            if perms.startswith("d") and depth < max_depth:
                dirs_to_visit.append((full_child, depth + 1))
            elif perms.startswith("-") and name.lower().endswith(VIDEO_EXTENSIONS):
                try:
                    size_bytes = int(size_str)
                except ValueError:
                    size_bytes = 0
                results.append((full_child, name, size_bytes))

    return results


def write_batch_queue_state(total: int, completed: int, failed: int, active_workers: int) -> None:
    """Persists volatile batch queue telemetry to disk for dbsrt top and monitoring."""
    try:
        q_path = Path(DEFAULT_BATCH_QUEUE_PATH)
        q_path.parent.mkdir(parents=True, exist_ok=True)
        rem = max(0, total - (completed + failed))
        pct = round(((completed + failed) / total * 100.0), 1) if total > 0 else 0.0
        state = {
            "total_targets": total,
            "completed_targets": completed,
            "failed_targets": failed,
            "remaining_targets": rem,
            "progress_pct": pct,
            "active_workers": active_workers,
            "is_active": rem > 0 and active_workers > 0,
            "updated_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        }
        tmp_path = q_path.with_suffix(".tmp")
        tmp_path.write_text(json.dumps(state, indent=2), encoding="utf-8")
        tmp_path.replace(q_path)
    except Exception:
        pass


def _get_process_tree_cpu_ticks(pid: int) -> int:
    """Computes total cumulative CPU user+system ticks for process and all direct children."""
    total_ticks = 0
    try:
        pids = [pid]
        children_path = f"/proc/{pid}/task/{pid}/children"
        if os.path.exists(children_path):
            with open(children_path, encoding="utf-8") as cf:
                for child in cf.read().split():
                    try:
                        pids.append(int(child))
                    except ValueError:
                        pass
        for p in pids:
            stat_path = f"/proc/{p}/stat"
            if os.path.exists(stat_path):
                with open(stat_path, encoding="utf-8") as sf:
                    content = sf.read()
                    r_idx = content.rfind(")")
                    if r_idx != -1:
                        fields = content[r_idx + 2 :].split()
                        total_ticks += int(fields[11]) + int(fields[12])
    except Exception:
        pass
    return total_ticks


def process_single_video(
    video_info: Tuple[str, str, int],
    stream_base: str,
    protocol: str,
    dbsrt_bin: str,
    log_lock: threading.Lock,
    events_log: Path,
    max_extraction_secs: int = DEFAULT_MAX_EXTRACTION_SECS,
) -> Dict[str, object]:
    """Processes a single video by invoking dbsrt media:add directly over FTP or HTTP bridge."""
    rel_path, filename, file_size = video_info
    stem = Path(filename).stem

    unquoted = urllib.parse.unquote(rel_path)
    if not unquoted.startswith("/"):
        unquoted = "/" + unquoted

    if protocol == "ftp":
        # Direct FTP: FFmpeg libavformat/ftp.c expects verbatim raw paths without URL encoding
        stream_url = f"{stream_base}{unquoted}"
    else:
        # HTTP bridge: Percent-encode URL path for standard HTTP routing
        stream_path_enc = urllib.parse.quote(unquoted, safe="/")
        stream_url = f"{stream_base}{stream_path_enc}"

    env = os.environ.copy()
    bin_dir = str(Path.home() / "dbsrt" / "bin")
    env["PATH"] = f"{bin_dir}:{env.get('PATH', '')}"

    base_cmd = [
        dbsrt_bin,
        "media:add",
        stream_url,
        "--media-name",
        urllib.parse.unquote(filename),
        "--hwaccel",
        "disabled",
        "--phash-dir",
        DEFAULT_PHASH_DIR,
        "--subtitles-dir",
        DEFAULT_SUBTITLES_DIR,
        "--skip-certify",
    ]

    with STREAM_BYTES_LOCK:
        start_stream_bytes = STREAM_BYTES_MAP.get(rel_path, 0)

    start_time = time.time()
    start_iso = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(start_time))
    print(f"--> [DEBUG] Launching dbsrt for {filename}...", flush=True)

    log_path = Path("logs") / "workers" / f"{stem}.log"
    log_path.parent.mkdir(parents=True, exist_ok=True)
    success = False
    returncode = -1
    try:
        with open(log_path, "w", encoding="utf-8") as out_f:
            proc = subprocess.Popen(
                base_cmd,
                stdout=out_f,
                stderr=subprocess.STDOUT,
                env=env,
                start_new_session=True,
            )
            last_bytes = start_stream_bytes
            last_cpu_ticks = 0
            stall_duration = 0
            while True:
                ret = proc.poll()
                if ret is not None:
                    returncode = ret
                    success = returncode == 0
                    break

                time.sleep(1.0)
                now_elapsed = time.time() - start_time

                has_progress = False
                if protocol != "ftp":
                    with STREAM_BYTES_LOCK:
                        cur_bytes = STREAM_BYTES_MAP.get(rel_path, 0)
                    if cur_bytes != last_bytes:
                        has_progress = True
                        last_bytes = cur_bytes
                else:
                    cur_cpu_ticks = _get_process_tree_cpu_ticks(proc.pid)
                    if cur_cpu_ticks != last_cpu_ticks:
                        has_progress = True
                        last_cpu_ticks = cur_cpu_ticks

                if has_progress:
                    stall_duration = 0
                else:
                    stall_duration += 1

                # Stall detector: zero throughput / zero CPU activity for >= DEFAULT_STALL_TIMEOUT_SECS
                if stall_duration >= DEFAULT_STALL_TIMEOUT_SECS:
                    try:
                        os.killpg(proc.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    returncode = -15
                    out_f.write(
                        f"\n❌ [WATCHDOG] Killed process: zero progress / activity for {stall_duration}s (Stall Detector).\n"
                    )
                    out_f.flush()
                    break

                # Max ceiling duration exceeded
                if now_elapsed >= max_extraction_secs:
                    try:
                        os.killpg(proc.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    returncode = -9
                    out_f.write(f"\n❌ [WATCHDOG] Killed process: exceeded max duration of {max_extraction_secs}s.\n")
                    out_f.flush()
                    break
    except Exception as exc:
        with open(log_path, "a", encoding="utf-8") as out_f:
            out_f.write(f"\n❌ Execution exception: {exc}\n")

    print(f"--> [DEBUG] dbsrt returned {returncode} for {filename}", flush=True)

    end_time = time.time()
    end_iso = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(end_time))
    wall_duration = end_time - start_time

    with STREAM_BYTES_LOCK:
        streamed_bytes = STREAM_BYTES_MAP.get(rel_path, 0) - start_stream_bytes

    # Physical network/CIFS throughput strictly based on actual streamed bytes over the wire
    if protocol == "ftp":
        actual_bytes = file_size if success else 0
        streamed_bytes = actual_bytes
    else:
        actual_bytes = streamed_bytes if streamed_bytes > 0 else (file_size if not stream_url.startswith("http") else 0)
    rate_mib_s = (actual_bytes / (1024 * 1024)) / wall_duration if wall_duration > 0 else 0.0

    record: Dict[str, object] = {
        "video_stem": stem,
        "video_rel_path": rel_path,
        "file_size_bytes": file_size,
        "streamed_bytes": streamed_bytes,
        "wall_duration_secs": round(wall_duration, 2),
        "io_read_rate_mib_s": round(rate_mib_s, 2),
        "cifs_read_rate_mib_s": round(rate_mib_s, 2),
        "start_iso": start_iso,
        "end_iso": end_iso,
        "timestamp_iso": end_iso,
        "success": success,
        "status": "COMPLETED"
        if success
        else ("STALLED" if returncode == -15 else ("TIMEOUT" if returncode == -9 else "ERROR")),
    }
    if not success and log_path.exists():
        record["error_summary"] = log_path.read_text(encoding="utf-8", errors="ignore")[-300:]

    with log_lock:
        events_log.parent.mkdir(parents=True, exist_ok=True)
        with open(events_log, "a", encoding="utf-8") as f:
            f.write(json.dumps(record) + "\n")

    return record


def main() -> int:
    """CLI Entry point for o2switch batch processing."""
    parser = argparse.ArgumentParser(description="o2switch Remote Batch Processing Worker")
    parser.add_argument("--workers", type=int, default=DEFAULT_WORKERS, help="Parallel workers")
    parser.add_argument(
        "--protocol", choices=["ftp", "sftp-bridge"], default="ftp", help="Streaming protocol (ftp or sftp-bridge)"
    )
    parser.add_argument("--ftp-port", type=int, default=21, help="NAS FTP port (default: 21)")
    parser.add_argument("--fresh-only", action="store_true", help="Process only virgin unindexed videos")
    parser.add_argument("--retry-failed", action="store_true", help="Retry videos that previously failed in events log")
    parser.add_argument(
        "--max-extraction-secs",
        type=int,
        default=DEFAULT_MAX_EXTRACTION_SECS,
        help=f"Maximum extraction wall duration in seconds before watchdog kill (default: {DEFAULT_MAX_EXTRACTION_SECS}s)",
    )
    parser.add_argument("--limit", type=int, default=None, help="Maximum number of videos to process")
    parser.add_argument("--port", type=int, default=DEFAULT_BRIDGE_PORT, help="Bridge HTTP port")
    parser.add_argument("--nas-host", default=DEFAULT_NAS_HOST, help="Synology NAS DDNS or IP")
    parser.add_argument("--nas-user", default=DEFAULT_NAS_USER, help="Synology NAS SFTP username")
    parser.add_argument("--nas-pass", default=os.environ.get("NAS_PASS", ""), help="Synology NAS password")
    parser.add_argument("--nas-base", default=DEFAULT_NAS_BASE, help="NAS base directory")
    parser.add_argument(
        "--targets-file",
        default="",
        help="Optional path to JSON file containing list of target video paths",
    )
    args = parser.parse_args()

    if not args.nas_pass:
        pass_file = Path("etc/.nas_pass")
        if pass_file.is_file():
            args.nas_pass = pass_file.read_text(encoding="utf-8").strip()
        elif "NAS_PASS" in _CONFIG_ENV:
            args.nas_pass = _CONFIG_ENV["NAS_PASS"]

    if not args.nas_pass:
        print(
            "❌ Error: Synology NAS password required via --nas-pass, NAS_PASS env or etc/.nas_pass.", file=sys.stderr
        )
        return 1

    dbsrt_bin = "./bin/dbsrt" if Path("./bin/dbsrt").exists() else "dbsrt"
    couplings_file = Path(DEFAULT_COUPLINGS_PATH)
    phash_dir = Path(DEFAULT_PHASH_DIR)
    events_log = Path(DEFAULT_EVENTS_LOG_PATH)

    print("=== [1/4] Scanning local catalogs and couplings ===")
    certified_stems = load_certified_stems(couplings_file)
    existing_phash = load_existing_phash_stems(phash_dir)
    print(f"✓ Found {len(certified_stems)} certified stems and {len(existing_phash)} .phash files locally.")

    targets: List[Tuple[str, str, int]] = []
    targets_path = Path(args.targets_file) if args.targets_file else Path("data/reports/selected_batch_targets.json")
    if targets_path.is_file():
        print(f"=== [2/4] Loading explicit targets from {targets_path} ===")
        try:
            targets_data = json.loads(targets_path.read_text(encoding="utf-8"))
            for item in targets_data:
                if isinstance(item, dict):
                    p_str = item.get("path", "")
                    filename = item.get("filename", Path(p_str).name)
                    stem = item.get("stem", Path(p_str).stem)
                    size_bytes = item.get("size_bytes", 0)
                else:
                    p_str = str(item)
                    filename = Path(p_str).name
                    stem = Path(p_str).stem
                    size_bytes = 0

                idx = p_str.find(args.nas_base)
                rel_path = p_str[idx:] if idx != -1 else p_str
                if args.fresh_only:
                    if stem in certified_stems or stem in existing_phash:
                        continue
                targets.append((rel_path, filename, size_bytes))
            print(f"✓ Loaded {len(targets)} targets from {targets_path}.")
        except Exception as err:
            print(f"⚠️ Failed reading {targets_path}: {err}. Falling back to remote scan.", file=sys.stderr)

    if not targets and args.retry_failed:
        print(f"=== [2/4] Resolving failed episodes from {events_log} ===")
        failed_targets_map: Dict[str, Tuple[str, str, int]] = {}
        success_stems: Set[str] = set()
        if events_log.is_file():
            try:
                for line in events_log.read_text(encoding="utf-8").splitlines():
                    if not line.strip():
                        continue
                    rec = json.loads(line)
                    stem = rec.get("video_stem", "")
                    rel_path = rec.get("video_rel_path", "")
                    if rec.get("success", False):
                        success_stems.add(stem)
                    elif stem and rel_path:
                        filename = Path(rel_path).name
                        size_bytes = rec.get("file_size_bytes", 0)
                        failed_targets_map[stem] = (rel_path, filename, size_bytes)
            except Exception as err:
                print(f"⚠️ Error reading {events_log}: {err}", file=sys.stderr)

        for stem, item in failed_targets_map.items():
            if stem not in success_stems and stem not in existing_phash and stem not in certified_stems:
                targets.append(item)
        print(f"✓ Found {len(targets)} failed videos to retry.")

    if not targets:
        print(f"=== [2/4] Discovering remote NAS video library ({args.nas_host}:{args.nas_base}) ===")
        if args.protocol == "ftp":
            all_videos = list_nas_videos_ftp(
                nas_host=args.nas_host,
                nas_user=args.nas_user,
                nas_pass=args.nas_pass,
                base_dir=args.nas_base,
                ftp_port=args.ftp_port,
            )
        else:
            all_videos = list_nas_videos_recursive(
                nas_host=args.nas_host,
                nas_user=args.nas_user,
                nas_pass=args.nas_pass,
                base_dir=args.nas_base,
            )
        print(f"✓ Discovered {len(all_videos)} video files on NAS.")

        for rel_path, filename, size_bytes in all_videos:
            stem = Path(filename).stem
            if args.fresh_only:
                if stem in certified_stems or stem in existing_phash:
                    continue
            targets.append((rel_path, filename, size_bytes))

    if args.limit and len(targets) > args.limit:
        targets = targets[: args.limit]

    print(f"✓ Selected {len(targets)} target videos to process (fresh_only={args.fresh_only}, limit={args.limit}).")
    if not targets:
        print("🎉 No target videos to process. Library is up-to-date!")
        return 0

    for rel_path, _filename, size_bytes in targets:
        unq = rel_path if rel_path.startswith("/") else "/" + rel_path
        FILE_SIZES_MAP[unq] = size_bytes

    if args.protocol == "ftp":
        print(f"=== [3/4] Direct FTP Native Streaming Mode ({args.nas_host}:{args.ftp_port}) ===")
        nas_pass_encoded = urllib.parse.quote(args.nas_pass)
        stream_base = f"ftp://{args.nas_user}:{nas_pass_encoded}@{args.nas_host}:{args.ftp_port}"
        server = None
        print("✓ Zero-Proxy Direct FTP mode active (native FFmpeg libavformat C streaming).")
    else:
        print(f"=== [3/4] Starting In-Memory HTTP Streaming Bridge on 127.0.0.1:{args.port} ===")
        StreamingBridgeHandler.nas_host = args.nas_host
        StreamingBridgeHandler.nas_user = args.nas_user
        StreamingBridgeHandler.nas_pass = args.nas_pass

        server = ThreadedHTTPServer(("127.0.0.1", args.port), StreamingBridgeHandler)
        server_thread = threading.Thread(target=server.serve_forever, daemon=True)
        server_thread.start()
        stream_base = f"http://127.0.0.1:{args.port}/stream"
        print("✓ Bridge running in background (Zero-NVMe direct streaming active).")

    print(f"=== [4/4] Executing Batch Ingestion ({args.workers} parallel workers) ===")
    log_lock = threading.Lock()
    success_count = 0
    fail_count = 0
    write_batch_queue_state(len(targets), success_count, fail_count, args.workers)

    start_batch = time.time()
    try:
        with ThreadPoolExecutor(max_workers=args.workers) as executor:
            future_map = {
                executor.submit(
                    process_single_video,
                    target,
                    stream_base,
                    args.protocol,
                    dbsrt_bin,
                    log_lock,
                    events_log,
                    args.max_extraction_secs,
                ): target
                for target in targets
            }
            for future in as_completed(future_map):
                target = future_map[future]
                try:
                    rec = future.result()
                    if rec.get("success"):
                        success_count += 1
                        print(
                            f"  ✓ [{success_count}/{len(targets)}] {rec['video_stem']} ({rec['wall_duration_secs']}s, {rec['io_read_rate_mib_s']} MB/s)"
                        )
                    else:
                        fail_count += 1
                        print(
                            f"  ❌ [{fail_count} FAILED] {rec['video_stem']}: {rec.get('error_summary', 'Unknown error')}"
                        )
                except Exception as exc:
                    fail_count += 1
                    print(f"  ❌ Exception processing {target[1]}: {exc}")
                write_batch_queue_state(len(targets), success_count, fail_count, args.workers)
    finally:
        write_batch_queue_state(len(targets), success_count, fail_count, 0)
        if server is not None:
            try:
                server.shutdown()
            except Exception:
                pass
            try:
                server.server_close()
            except Exception:
                pass

    total_duration = time.time() - start_batch
    print("=" * 60)
    print(f"🏁 Batch completed in {total_duration:.1f}s: {success_count} succeeded, {fail_count} failed.")
    return 0 if fail_count == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
