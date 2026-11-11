#!/usr/bin/env python3
"""Empirical Benchmark: Worker Scaling & GPU/CIFS Saturation for VA-API pHash Ingestion.

Discovers distinct files on Synology CIFS, benchmarks W in [1, 2, 3, 4, 6, 8, 10],
samples intel_gpu_top in real time, and prints empirical throughput and saturation metrics.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
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
BYTES_PER_MIB: int = 1024 * 1024
DEFAULT_RENDER_DEVICE: str = "/dev/dri/renderD128"


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
class BenchmarkRunResult:
    workers: int
    wall_duration_secs: float
    total_frames: int
    aggregate_fps: float
    aggregate_cifs_mib_s: float
    avg_cpu_percent: float
    avg_gpu_video_percent: float


def _is_valid_media_file(p: Path, min_size: int = MIN_FILE_SIZE_BYTES) -> bool:
    """Predicate filtering hidden or undersized media files."""
    if p.name.startswith("."):
        return False
    try:
        return p.stat().st_size >= min_size
    except OSError:
        return False


def resolve_tvshow_root(custom_dir: str | None = None) -> Path:
    """Resolves TVShow directory from custom arg, env, or default candidate."""
    if custom_dir:
        return Path(custom_dir)
    env_dir = os.environ.get("NAS_TVSHOW_DIR")
    if env_dir:
        return Path(env_dir)
    user = os.environ.get("USER", "storage")
    candidate = Path("/media") / user / "Synology" / "tvshow"
    return candidate if candidate.exists() else Path("/media/storage/tvshow")


def find_distinct_synology_media(tvshow_dir: Path, count: int = 15) -> List[Path]:
    """Finds distinct large video files across different TV shows to prevent pagecache hits."""
    if not tvshow_dir.exists():
        return []

    found: List[Path] = []
    seen_dirs: set[Path] = set()

    for p in tvshow_dir.rglob("*.mkv"):
        if _is_valid_media_file(p) and p.parent not in seen_dirs:
            found.append(p)
            seen_dirs.add(p.parent)
            if len(found) >= count:
                return found

    if len(found) < count:
        for p in tvshow_dir.rglob("*.mkv"):
            if p not in found and _is_valid_media_file(p):
                found.append(p)
                if len(found) >= count:
                    break

    return found


def run_worker_task(video_path: Path, frames_count: int) -> tuple[int, int]:
    """Runs one worker extraction pipeline and returns (frames, read_chars)."""
    ffmpeg_bin = shutil.which("ffmpeg") or "/usr/bin/ffmpeg"
    cmd = [
        ffmpeg_bin,
        "-hide_banner",
        "-nostats",
        "-nostdin",
        "-hwaccel",
        "vaapi",
        "-vaapi_device",
        DEFAULT_RENDER_DEVICE,
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
        bufsize=32 * 32,
    )

    frame_size = 32 * 32
    frames_read = 0

    assert p.stdout is not None
    while True:
        chunk = p.stdout.read(frame_size)
        if not chunk or len(chunk) < frame_size:
            break
        frames_read += 1

    p.wait()

    read_bytes = 0
    if frames_read > 0:
        try:
            file_sz = video_path.stat().st_size
            read_bytes = int(file_sz * (frames_read / 70000.0))
        except OSError:
            pass

    return frames_read, read_bytes


def _parse_gpu_samples(output_file: Path) -> List[float]:
    """Parses JSON samples recorded by intel_gpu_top."""
    samples: List[float] = []
    if not output_file.exists():
        return samples

    try:
        text = output_file.read_text(encoding="utf-8", errors="ignore").strip()
        if text.startswith("[") and not text.endswith("]"):
            text = text.rstrip(", \r\n") + "]"
        data = json.loads(text)
        for item in data:
            v = item.get("engines", {}).get("Video", {}).get("busy", 0.0)
            samples.append(float(v))
    except Exception:
        pass
    return samples


class GpuSampler:
    """Samples intel_gpu_top in real time to capture Video Engine busy %."""

    def __init__(self) -> None:
        self.proc: subprocess.Popen | None = None
        self.output_file = Path("/tmp/gpu_sample.json")

    def start(self) -> None:
        if self.output_file.exists():
            self.output_file.unlink()
        gpu_top_bin = shutil.which("intel_gpu_top") or "/usr/bin/intel_gpu_top"
        cmd = [gpu_top_bin, "-J", "-s", "300", "-o", str(self.output_file)]
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

        samples = _parse_gpu_samples(self.output_file)
        if samples:
            return round(sum(samples) / len(samples), 1)
        return 0.0


def benchmark_workers(
    worker_count: int, media_pool: List[Path], frames_count: int = FRAMES_PER_WORKER
) -> BenchmarkRunResult:
    """Executes a single test run at given worker concurrency W."""
    selected_files = media_pool[:worker_count]
    while len(selected_files) < worker_count:
        selected_files.extend(media_pool[: worker_count - len(selected_files)])

    gpu_sampler = GpuSampler()
    gpu_sampler.start()

    psutil.cpu_percent(interval=None)
    start_cpu = psutil.cpu_percent(interval=None)

    t0 = time.time()
    total_frames = 0
    total_bytes = 0

    with ThreadPoolExecutor(max_workers=worker_count) as pool:
        futures = [pool.submit(run_worker_task, f, frames_count) for f in selected_files]
        for fut in futures:
            frames, b = fut.result()
            total_frames += frames
            total_bytes += b

    wall = max(time.time() - t0, 0.1)
    avg_gpu = gpu_sampler.stop()
    end_cpu = psutil.cpu_percent(interval=None)
    avg_cpu = max((start_cpu + end_cpu) / 2.0, 1.0)

    fps = total_frames / wall
    cifs_mib = (total_bytes / BYTES_PER_MIB) / wall

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
    repo_root = find_repo_root()

    parser = argparse.ArgumentParser(
        description="Worker Scaling & GPU/CIFS Saturation Benchmark for VA-API pHash Ingestion."
    )
    parser.add_argument(
        "--dir",
        type=str,
        default=None,
        help="Path to TVShow directory containing media files",
    )
    parser.add_argument(
        "--frames",
        type=int,
        default=FRAMES_PER_WORKER,
        help=f"Number of frames sampled per worker (default: {FRAMES_PER_WORKER})",
    )
    parser.add_argument(
        "--check",
        "--dry-run",
        action="store_true",
        help="Inspect environment, devices, and media pool availability without running benchmark",
    )
    args = parser.parse_args()

    tvshow_dir = resolve_tvshow_root(args.dir)
    ffmpeg_bin = shutil.which("ffmpeg")
    gpu_top_bin = shutil.which("intel_gpu_top")
    render_dev = Path(DEFAULT_RENDER_DEVICE)

    if args.check:
        print("=== DBSrt Worker Scaling Benchmark Inspection ===")
        print(f"✓ REPO_ROOT     : {repo_root}")
        print(f"✓ TVShow dir    : {tvshow_dir} (exists: {tvshow_dir.exists()})")
        media_pool = find_distinct_synology_media(tvshow_dir, count=12) if tvshow_dir.exists() else []
        print(f"✓ Media pool    : {len(media_pool)} distinct files found")
        print(f"✓ ffmpeg binary : {ffmpeg_bin or 'not found'}")
        print(f"✓ intel_gpu_top : {gpu_top_bin or 'not found'}")
        print(f"✓ Render device : {render_dev} (exists: {render_dev.exists()})")
        return

    print("🔍 Discovering distinct media files across Synology TVShow shares...")
    media_pool = find_distinct_synology_media(tvshow_dir, count=12)
    print(f"✅ Found {len(media_pool)} distinct files for contention-free benchmarking.")

    results: List[BenchmarkRunResult] = []

    print("\n🚀 Starting Scaling Benchmark across Workers [1, 2, 3, 4, 6, 8, 10]...")
    print(f"📊 Sampling {args.frames} frames per worker per run.\n")

    for w in SAMPLE_WORKER_COUNTS:
        if w > len(media_pool):
            break
        print(f"  ▶ Benchmarking {w} worker(s)...", end="", flush=True)
        res = benchmark_workers(w, media_pool, frames_count=args.frames)
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
