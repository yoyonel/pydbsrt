#!/usr/bin/env python3
"""Empirical Benchmark: Worker Scaling & GPU/CIFS Saturation for VA-API pHash Ingestion.

Discovers distinct files on Synology CIFS, benchmarks W in [1, 2, 3, 4, 6, 8, 10],
samples intel_gpu_top in real time, and prints empirical throughput and saturation metrics.
"""

from __future__ import annotations

import json
import subprocess
import time
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path
from typing import List

import psutil

FRAMES_PER_WORKER: int = 1200
SAMPLE_WORKER_COUNTS: List[int] = [1, 2, 3, 4, 6, 8, 10]
MIN_FILE_SIZE_BYTES: int = 800 * 1024 * 1024  # 800 MB


@dataclass
class BenchmarkRunResult:
    workers: int
    wall_duration_secs: float
    total_frames: int
    aggregate_fps: float
    aggregate_cifs_mib_s: float
    avg_cpu_percent: float
    avg_gpu_video_percent: float


def find_distinct_synology_media(count: int = 15) -> List[Path]:
    """Finds distinct large video files across different TV shows to prevent pagecache hits."""
    tvshow = Path("/media/latty/Synology/tvshow")
    found: List[Path] = []
    seen_dirs: set[Path] = set()

    for p in tvshow.rglob("*.mkv"):
        if p.name.startswith("."):
            continue
        try:
            if p.parent not in seen_dirs and p.stat().st_size >= MIN_FILE_SIZE_BYTES:
                found.append(p)
                seen_dirs.add(p.parent)
                if len(found) >= count:
                    break
        except OSError:
            continue

    if len(found) < count:
        # Relax dir constraint if needed
        for p in tvshow.rglob("*.mkv"):
            if p not in found and not p.name.startswith("."):
                try:
                    if p.stat().st_size >= MIN_FILE_SIZE_BYTES:
                        found.append(p)
                        if len(found) >= count:
                            break
                except OSError:
                    continue

    return found


def run_worker_task(video_path: Path, frames_count: int) -> tuple[int, int]:
    """Runs one worker extraction pipeline and returns (frames, read_chars)."""
    cmd = [
        "/usr/bin/ffmpeg",
        "-hide_banner",
        "-nostats",
        "-nostdin",
        "-hwaccel",
        "vaapi",
        "-vaapi_device",
        "/dev/dri/renderD128",
        "-hwaccel_output_format",
        "vaapi",
        "-i",
        str(video_path),
        "-frames:v",
        str(frames_count),
        "-vf",
        "scale_vaapi=w=32:h=32,hwdownload,format=nv12,format=gray",
        "-f",
        "rawvideo",
        "-",
    ]

    p = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        bufsize=1024 * 1024,
    )

    proc_psutil = None
    try:
        proc_psutil = psutil.Process(p.pid)
    except (psutil.NoSuchProcess, psutil.AccessDenied):
        pass

    read_bytes = 0
    frames_read = 0
    buf_size = 1024  # 32x32 gray

    if p.stdout:
        while True:
            chunk = p.stdout.read(buf_size)
            if not chunk or len(chunk) < buf_size:
                break
            frames_read += 1

    p.wait()

    if proc_psutil:
        try:
            io = proc_psutil.io_counters()
            read_bytes = max(getattr(io, "read_chars", 0), getattr(io, "read_bytes", 0))
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            pass

    # If proc io already disappeared, estimate from file bitrate
    if read_bytes == 0 and frames_read > 0:
        try:
            file_sz = video_path.stat().st_size
            # Rough proportional estimate for 1200 frames out of ~70000
            read_bytes = int(file_sz * (frames_read / 70000.0))
        except OSError:
            pass

    return frames_read, read_bytes


class GpuSampler:
    """Samples intel_gpu_top in real time to capture Video Engine busy %."""

    def __init__(self) -> None:
        self.proc: subprocess.Popen | None = None
        self.output_file = Path("/tmp/gpu_sample.json")

    def start(self) -> None:
        if self.output_file.exists():
            self.output_file.unlink()
        cmd = ["/usr/bin/intel_gpu_top", "-J", "-s", "300", "-o", str(self.output_file)]
        self.proc = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def stop(self) -> float:
        if self.proc:
            try:
                self.proc.terminate()
                self.proc.wait(timeout=2.0)
            except Exception:
                try:
                    self.proc.kill()
                except Exception:
                    pass

        # Parse collected JSON samples
        samples: List[float] = []
        if self.output_file.exists():
            try:
                text = self.output_file.read_text(encoding="utf-8", errors="ignore")
                # Clean trailing commas for JSON array parsing
                cleaned = text.strip()
                if cleaned.startswith("[") and not cleaned.endswith("]"):
                    cleaned = cleaned.rstrip(", \r\n") + "]"
                data = json.loads(cleaned)
                for item in data:
                    v = item.get("engines", {}).get("Video", {}).get("busy", 0.0)
                    if isinstance(v, int | float):
                        samples.append(float(v))
            except Exception:
                pass
            try:
                self.output_file.unlink()
            except OSError:
                pass

        return round(sum(samples) / len(samples), 1) if samples else 0.0


def benchmark_workers(worker_count: int, media_pool: List[Path]) -> BenchmarkRunResult:
    """Benchmarks a specific worker count across distinct files."""
    selected_files = media_pool[:worker_count]
    gpu_sampler = GpuSampler()

    start_cpu = psutil.cpu_percent(interval=None)
    gpu_sampler.start()
    t0 = time.time()

    total_frames = 0
    total_bytes = 0

    with ThreadPoolExecutor(max_workers=worker_count) as pool:
        futures = [pool.submit(run_worker_task, f, FRAMES_PER_WORKER) for f in selected_files]
        for fut in futures:
            frames, b = fut.result()
            total_frames += frames
            total_bytes += b

    wall = max(time.time() - t0, 0.1)
    avg_gpu = gpu_sampler.stop()
    end_cpu = psutil.cpu_percent(interval=None)
    avg_cpu = max((start_cpu + end_cpu) / 2.0, 1.0)

    fps = total_frames / wall
    cifs_mib = (total_bytes / (1024 * 1024)) / wall

    return BenchmarkRunResult(
        workers=worker_count,
        wall_duration_secs=round(wall, 2),
        total_frames=total_frames,
        aggregate_fps=round(fps, 1),
        aggregate_cifs_mib_s=round(cifs_mib, 1),
        avg_cpu_percent=round(avg_cpu, 1),
        avg_gpu_video_percent=avg_gpu,
    )


def main() -> None:
    print("🔍 Discovering distinct media files across Synology TVShow shares...")
    media_pool = find_distinct_synology_media(count=12)
    print(f"✅ Found {len(media_pool)} distinct files for contention-free benchmarking.")

    results: List[BenchmarkRunResult] = []

    print("\n🚀 Starting Scaling Benchmark across Workers [1, 2, 3, 4, 6, 8, 10]...")
    print(f"📊 Sampling {FRAMES_PER_WORKER} frames per worker per run.\n")

    for w in SAMPLE_WORKER_COUNTS:
        if w > len(media_pool):
            break
        print(f"  ▶ Benchmarking {w} worker(s)...", end="", flush=True)
        res = benchmark_workers(w, media_pool)
        results.append(res)
        print(
            f" Done in {res.wall_duration_secs}s -> Aggregate: {res.aggregate_fps} FPS | CIFS: {res.aggregate_cifs_mib_s} MB/s | GPU Video: {res.avg_gpu_video_percent}% | Host CPU: {res.avg_cpu_percent}%"
        )
        time.sleep(1.0)

    # Print summary table
    print("\n" + "=" * 80)
    print("📈 SYNTHÈSE EMPIRIQUE DU SCALING WORKERS (DÉCODAGE VA-API INTEL IRIS XE)")
    print("=" * 80)
    print(
        f"{'Workers':<9} | {'Durée (s)':<10} | {'Cadence FPS':<12} | {'Débit CIFS':<12} | {'GPU Video %':<12} | {'Host CPU %':<10}"
    )
    print("-" * 80)
    for r in results:
        print(
            f"{r.workers:<9} | {r.wall_duration_secs:<10} | {r.aggregate_fps:<12} | {r.aggregate_cifs_mib_s:<12} | {r.avg_gpu_video_percent:<12} | {r.avg_cpu_percent:<10}"
        )
    print("=" * 80)


if __name__ == "__main__":
    main()
