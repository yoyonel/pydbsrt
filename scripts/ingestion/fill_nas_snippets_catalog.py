#!/usr/bin/env python3
"""Fills missing local 15-second video snippets from Synology NAS and Freebox.

Ensures maximum corpus coverage of `catalog.dbphash` (3,061 medias) by streaming
and extracting 15s clips at middle (t=50%) directly over FTP or HTTP Byte-Range gateway
without downloading full video containers, and computing their accompanying `.phash` fingerprints.
Provides exhaustive error, timeout, and exception tracking with structured JSON reporting.
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import posixpath
import re
import shutil
import struct
import subprocess
import time
import unicodedata
import urllib.parse
from concurrent.futures import ThreadPoolExecutor, as_completed
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Dict, List, Set, Tuple


def find_repo_root() -> Path:
    """Finds repository root dynamically with appliance fallback."""
    curr = Path(__file__).resolve().parent
    for parent in [curr, *curr.parents]:
        if (parent / ".git").is_dir():
            return parent
    appliance_path = Path.home() / "dbsrt"
    if appliance_path.is_dir():
        return appliance_path
    return Path.cwd()


REPO_ROOT = find_repo_root()


def resolve_dbsrt_bin(repo_root: Path) -> Path:
    """Finds dbsrt binary in release, debug or PATH."""
    release = repo_root / "target/release/dbsrt"
    if release.exists():
        return release
    debug = repo_root / "target/debug/dbsrt"
    if debug.exists():
        return debug
    system = shutil.which("dbsrt")
    if system:
        return Path(system)
    return release


DEFAULT_NAS_HOST = os.environ.get("NAS_HOST", "yoyonel.synology.me")
DEFAULT_NAS_USER = os.environ.get("NAS_USER", "qzsl2686")
DEFAULT_PASS_ENTRY = os.environ.get("NAS_PASS_ENTRY", "nas/qzsl2686")
DEFAULT_CATALOG_PATH = REPO_ROOT / "data/catalog.dbphash"
DEFAULT_SNIPPETS_DIR = REPO_ROOT / "data/snippets"
DEFAULT_DBSRT_BIN = resolve_dbsrt_bin(REPO_ROOT)
DEFAULT_REPORT_PATH = REPO_ROOT / "data/reports/snippets_fill_report.json"
DEFAULT_FREEBOX_HTTP_PORT = 80


@dataclass
class MediaJob:
    media_id: int
    title: str
    duration_sec: float
    phash_name: str
    original_filename: str
    source_type: str  # "syno_ftp" | "freebox_http" | "unlocated"
    source_path: str


@dataclass
class JobResult:
    media_id: int
    title: str
    original_filename: str
    status: str
    source_type: str
    source_path: str
    elapsed_sec: float
    bytes_extracted: int = 0
    error_message: str | None = None
    error_code: int | None = None


def sanitize_filename(name: str) -> str:
    """Replaces unsafe filename characters with underscores."""
    clean = re.sub(r'[\\/*?:"<>|]', "_", name)
    clean = re.sub(r"\s+", "_", clean).strip("._")
    return clean[:120] if len(clean) > 120 else clean


def normalize_tokens(s: str) -> Set[str]:
    """Extracts lowercase alphanumeric tokens normalized via NFKD."""
    decomposed = unicodedata.normalize("NFKD", s)
    return {t for t in re.findall(r"[a-zA-Z0-9]+", decomposed.lower()) if len(t) > 1}


def resolve_nas_password(pass_entry: str = DEFAULT_PASS_ENTRY) -> str:
    """Resolves NAS password with explicit fallback and error diagnostics."""
    if os.environ.get("NAS_PASS"):
        return os.environ["NAS_PASS"]

    try:
        res = subprocess.run(["pass", "show", pass_entry], capture_output=True, text=True, timeout=5)
        if res.returncode == 0 and res.stdout.strip():
            return res.stdout.strip().splitlines()[0]
    except (subprocess.TimeoutExpired, OSError):
        pass

    try:
        res = subprocess.run(
            ["ssh", "o2switch", "cat ~/dbsrt/etc/.nas_pass"], capture_output=True, text=True, timeout=10
        )
        if res.returncode == 0 and res.stdout.strip():
            return res.stdout.strip()
    except (subprocess.TimeoutExpired, OSError):
        pass

    return ""


def load_catalog_entries(catalog_path: Path) -> List[Tuple[int, str, float]]:
    """Loads all media entries from binary .dbphash catalog."""
    if not catalog_path.is_file():
        return []

    with open(catalog_path, "rb") as fh:
        hdr = fh.read(128)
        if len(hdr) < 80:
            return []
        (
            _magic,
            _ver,
            _flags,
            media_count,
            _cuts,
            _sparse,
            off_media,
            _off_cuts,
            _off_sparse,
            off_str,
            str_len,
            _csum,
        ) = struct.unpack("<8sHH I Q Q Q Q Q Q Q Q", hdr[:80])

        fh.seek(off_str)
        str_table = fh.read(str_len)

        fh.seek(off_media)
        medias: List[Tuple[int, str, float]] = []
        for _ in range(media_count):
            entry = fh.read(64)
            if len(entry) < 36:
                break
            (
                m_id,
                _type,
                _fc,
                _year,
                _season,
                _ep,
                dur_ms,
                _fps_k,
                t_off,
                t_len,
            ) = struct.unpack("<IBBHIIQIII", entry[:36])
            title = str_table[t_off : t_off + t_len].decode("utf-8", errors="replace")
            medias.append((m_id, title, dur_ms / 1000.0))

    return medias


def load_phash_sidecars(phash_dir: Path) -> Dict[str, str]:
    """Loads JSON sidecars mapping moviehash -> media_name."""
    sidecars: Dict[str, str] = {}
    for jpath in phash_dir.glob("*.json"):
        try:
            with open(jpath, encoding="utf-8") as fh:
                name = json.load(fh).get("media_name")
                if name:
                    sidecars[jpath.stem.lower()] = name
        except (json.JSONDecodeError, OSError):
            continue
    return sidecars


def _load_tvshow_nightly_index() -> Dict[str, Tuple[str, str]]:
    index: Dict[str, Tuple[str, str]] = {}
    rpath = REPO_ROOT / "data/reports/batch_tvshow_nightly.jsonl"
    if not rpath.is_file():
        return index
    with open(rpath, encoding="utf-8") as fh:
        for line in fh:
            try:
                rel = json.loads(line).get("video_rel_path")
                if rel:
                    fname = posixpath.basename(rel)
                    index[fname] = ("syno_ftp", rel.lstrip("/"))
            except json.JSONDecodeError:
                continue
    return index


def _load_movies_targets_index() -> Dict[str, Tuple[str, str]]:
    index: Dict[str, Tuple[str, str]] = {}
    rpath = REPO_ROOT / "data/reports/selected_batch_targets.json"
    if not rpath.is_file():
        return index
    with open(rpath, encoding="utf-8") as fh:
        try:
            for t in json.load(fh):
                fname = t.get("filename")
                fpath = t.get("path", "")
                if fname and "/Synology/" in fpath:
                    index[fname] = ("syno_ftp", fpath.split("/Synology/", 1)[1].lstrip("/"))
        except json.JSONDecodeError:
            pass
    return index


def _load_freebox_targets_index() -> Dict[str, Tuple[str, str]]:
    index: Dict[str, Tuple[str, str]] = {}
    rpath = REPO_ROOT / "data/reports/freebox_batch_targets.json"
    if not rpath.is_file():
        return index
    with open(rpath, encoding="utf-8") as fh:
        try:
            for t in json.load(fh):
                fname = t.get("filename")
                if fname and t.get("path"):
                    index[fname] = ("freebox_http", t["path"])
        except json.JSONDecodeError:
            pass
    return index


def _load_nas_ftp_index() -> Dict[str, Tuple[str, str]]:
    """Loads indexed Synology NAS files from nas_ftp_index.json."""
    index: Dict[str, Tuple[str, str]] = {}
    cache = REPO_ROOT / "data/reports/nas_ftp_index.json"
    if not cache.is_file():
        return index
    try:
        with open(cache, encoding="utf-8") as fh:
            raw = json.load(fh)
            entries = raw.get("index", raw)
            for k, v in entries.items():
                index[k] = ("syno_ftp", v.lstrip("/"))
    except (json.JSONDecodeError, OSError):
        pass
    return index


def build_unified_storage_index() -> Dict[str, Tuple[str, str]]:
    """Builds unified mapping from filename to (source_type, relative_path)."""
    index = _load_tvshow_nightly_index()
    index.update(_load_movies_targets_index())
    index.update(_load_freebox_targets_index())
    index.update(_load_nas_ftp_index())
    return index


def map_catalog_to_storage_jobs(
    catalog_medias: List[Tuple[int, str, float]],
    phash_dir: Path,
    storage_index: Dict[str, Tuple[str, str]],
) -> List[MediaJob]:
    """Maps every catalog media 1:1 to its exact storage source and path."""
    sidecars = load_phash_sidecars(phash_dir)
    phash_files = sorted(Path(f).name for f in glob.glob(str(phash_dir / "*.phash")))

    jobs: List[MediaJob] = []
    for idx, (m_id, title, dur) in enumerate(catalog_medias):
        phash_name = phash_files[idx] if idx < len(phash_files) else ""
        m_hash = re.match(r"^([0-9a-fA-F]{16})\.([0-9.]+)fps\.phash$", phash_name)
        if m_hash:
            orig_fname = sidecars.get(m_hash.group(1).lower(), "")
        else:
            orig_fname = urllib.parse.unquote(re.sub(r"\.[0-9.]+fps\.phash$", "", phash_name))

        source_type = "unlocated"
        source_path = ""

        # Exact and unquoted resolution
        for candidate in (orig_fname, urllib.parse.unquote(orig_fname)):
            if candidate in storage_index:
                source_type, source_path = storage_index[candidate]
                break

        jobs.append(MediaJob(m_id, title, dur, phash_name, orig_fname, source_type, source_path))

    return jobs


def build_media_stream_url(
    job: MediaJob,
    nas_host: str,
    nas_user: str,
    nas_pass: str,
    freebox_port: int,
) -> Tuple[str | None, str | None]:
    """Builds valid streaming URL or returns diagnostic error if unsupported."""
    if job.source_type == "syno_ftp":
        if "|" in job.source_path:
            return None, "pipe char '|' unsupported in ffmpeg ftp url"
        enc_pass = urllib.parse.quote(nas_pass, safe="")
        return f"ftp://{nas_user}:{enc_pass}@{nas_host}/{job.source_path}", None

    if job.source_type == "freebox_http":
        quoted_path = urllib.parse.quote(job.source_path, safe="/")
        return f"http://{nas_host}:{freebox_port}/{quoted_path}", None

    return None, f"media '{job.original_filename}' unlocated on storage"


def execute_ffmpeg_snippet_extraction(
    stream_url: str,
    duration_sec: float,
    out_mp4: Path,
    timeout_sec: float = 45.0,
) -> Tuple[bool, str | None, int | None]:
    """Runs FFmpeg to extract 15s snippet with timeout and error capture."""
    dur = duration_sec if duration_sec > 30.0 else 600.0
    start_sec = max(0.0, (dur * 0.5) - 7.5)

    cmd = [
        "ffmpeg",
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-ss",
        f"{start_sec:.2f}",
        "-i",
        stream_url,
        "-t",
        "15.0",
        "-vf",
        "scale=-2:480",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-crf",
        "28",
        "-an",
        str(out_mp4),
    ]

    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout_sec)
        if proc.returncode == 0 and out_mp4.is_file() and out_mp4.stat().st_size > 1024:
            return True, None, 0
        err_msg = proc.stderr.strip()[-250:] if proc.stderr else "empty output file"
        return False, err_msg, proc.returncode
    except subprocess.TimeoutExpired:
        return False, f"ffmpeg execution timed out after {timeout_sec}s", 124
    except OSError as e:
        return False, f"system execution error: {e}", 127


def execute_freebox_remote_extraction(
    stream_url: str,
    duration_sec: float,
    out_mp4: Path,
    timeout_sec: float = 60.0,
) -> Tuple[bool, str | None, int | None]:
    """Streams Freebox snippet remotely via o2switch SSH pipe to bypass NAT restriction."""
    dur = duration_sec if duration_sec > 30.0 else 600.0
    start_sec = max(0.0, (dur * 0.5) - 7.5)

    cmd = [
        "ssh",
        "-o",
        "BatchMode=yes",
        "o2switch",
        (
            f"~/dbsrt/bin/ffmpeg -hide_banner -loglevel error -y -ss {start_sec:.2f} "
            f'-i "{stream_url}" -t 15.0 -vf scale=-2:480 -c:v libx264 -preset ultrafast '
            "-crf 28 -an -f mp4 -movflags frag_keyframe+empty_moov pipe:1"
        ),
    ]

    try:
        proc = subprocess.run(cmd, capture_output=True, timeout=timeout_sec)
        if proc.returncode == 0 and len(proc.stdout) > 1024:
            with open(out_mp4, "wb") as fh:
                fh.write(proc.stdout)
            return True, None, 0
        err_msg = proc.stderr.decode("utf-8", errors="replace").strip()[-250:] if proc.stderr else "empty remote stream"
        return False, err_msg, proc.returncode
    except subprocess.TimeoutExpired:
        return False, f"remote o2switch ssh timed out after {timeout_sec}s", 124
    except OSError as e:
        return False, f"system ssh execution error: {e}", 127


def _dispatch_stream_extraction(
    job: MediaJob,
    stream_url: str,
    out_mp4: Path,
) -> Tuple[bool, str | None, int | None]:
    """Dispatches extraction to Freebox SSH pipe or local FFmpeg with retry."""
    err_msg: str | None = None
    err_code: int | None = None

    for _ in range(2):
        if job.source_type == "freebox_http":
            ok, err_msg, err_code = execute_freebox_remote_extraction(stream_url, job.duration_sec, out_mp4)
        else:
            ok, err_msg, err_code = execute_ffmpeg_snippet_extraction(stream_url, job.duration_sec, out_mp4)
        if ok:
            return True, None, 0
        time.sleep(1.0)

    return False, err_msg, err_code


def _export_snippet_phash(dbsrt_bin: Path, out_mp4: Path, out_phash: Path) -> None:
    """Computes accompanying .phash fingerprint using native dbsrt CLI."""
    if not dbsrt_bin.is_file():
        return
    cmd_phash = [str(dbsrt_bin), "export-phash", "-m", str(out_mp4), "-o", str(out_phash)]
    try:
        subprocess.run(cmd_phash, capture_output=True, timeout=30)
    except (subprocess.TimeoutExpired, OSError):
        pass


def extract_media_snippet(
    job: MediaJob,
    snippets_dir: Path,
    dbsrt_bin: Path,
    nas_host: str,
    nas_user: str,
    nas_pass: str,
    freebox_port: int,
) -> JobResult:
    """Executes single snippet extraction with exhaustive state reporting and retry."""
    t0 = time.monotonic()
    safe_stem = sanitize_filename(Path(job.source_path or job.original_filename).stem)
    out_mp4 = snippets_dir / f"{safe_stem}_mid15s.mp4"
    out_phash = snippets_dir / f"{safe_stem}_mid15s.phash"

    if out_mp4.is_file() and out_mp4.stat().st_size > 1024:
        return JobResult(
            job.media_id,
            job.title,
            job.original_filename,
            "CACHED",
            job.source_type,
            job.source_path,
            0.0,
            out_mp4.stat().st_size,
        )

    stream_url, url_err = build_media_stream_url(job, nas_host, nas_user, nas_pass, freebox_port)
    if not stream_url:
        st = "UNRESOLVED" if job.source_type == "unlocated" else "SKIPPED_URL"
        return JobResult(
            job.media_id, job.title, job.original_filename, st, job.source_type, job.source_path, 0.0, 0, url_err
        )

    ok, err_msg, err_code = _dispatch_stream_extraction(job, stream_url, out_mp4)
    elapsed = round(time.monotonic() - t0, 2)

    if not ok:
        st = "TIMEOUT" if err_code == 124 else ("ERROR_SSH" if job.source_type == "freebox_http" else "ERROR_FFMPEG")
        return JobResult(
            job.media_id,
            job.title,
            job.original_filename,
            st,
            job.source_type,
            job.source_path,
            elapsed,
            0,
            err_msg,
            err_code,
        )

    _export_snippet_phash(dbsrt_bin, out_mp4, out_phash)
    sz = out_mp4.stat().st_size if out_mp4.is_file() else 0
    return JobResult(
        job.media_id, job.title, job.original_filename, "EXTRACTED", job.source_type, job.source_path, elapsed, sz
    )


def parse_cli_args() -> argparse.Namespace:
    """Parses command line arguments."""
    parser = argparse.ArgumentParser(description="Fill missing snippets from NAS/Freebox covering catalog.dbphash.")
    parser.add_argument("--catalog", type=Path, default=DEFAULT_CATALOG_PATH, help="Path to catalog.dbphash")
    parser.add_argument(
        "--phash-dir", type=Path, default=REPO_ROOT / "data/__PHASH__", help="Directory containing .phash"
    )
    parser.add_argument("--snippets-dir", type=Path, default=DEFAULT_SNIPPETS_DIR, help="Destination directory")
    parser.add_argument("--workers", type=int, default=4, help="Parallel extraction workers (default: 4)")
    parser.add_argument("--limit", type=int, default=None, help="Maximum number of snippets to extract")
    parser.add_argument(
        "--check", "--dry-run", dest="dry_run", action="store_true", help="Audit inventory without extraction"
    )
    parser.add_argument("--report", type=Path, default=DEFAULT_REPORT_PATH, help="Output JSON report path")
    return parser.parse_args()


def _print_inventory_summary(jobs: List[MediaJob], snippets_dir: Path) -> None:
    cached = 0
    for j in jobs:
        safe_stem = sanitize_filename(Path(j.source_path or j.original_filename).stem)
        mp4 = snippets_dir / f"{safe_stem}_mid15s.mp4"
        if mp4.is_file() and mp4.stat().st_size > 1024:
            cached += 1

    syno_count = sum(1 for j in jobs if j.source_type == "syno_ftp")
    freebox_count = sum(1 for j in jobs if j.source_type == "freebox_http")
    unlocated = sum(1 for j in jobs if j.source_type == "unlocated")
    total = len(jobs)

    print("\n--------------------------------------------------------------------------------")
    print(f"  • Total Médias Catalogue : {total:4d} (100.0%)")
    print(f"  • Déjà Présents en Local : {cached:4d} ({(cached / max(1, total)) * 100:5.1f}%)")
    print(f"  • Localisés sur Synology : {syno_count:4d} ({(syno_count / max(1, total)) * 100:5.1f}%)")
    print(f"  • Localisés sur Freebox  : {freebox_count:4d} ({(freebox_count / max(1, total)) * 100:5.1f}%)")
    print(f"  • Non résolus / orphelins: {unlocated:4d} ({(unlocated / max(1, total)) * 100:5.1f}%)")
    print("--------------------------------------------------------------------------------")


def _run_worker_pool(
    candidates: List[MediaJob],
    args: argparse.Namespace,
    nas_pass: str,
) -> List[JobResult]:
    """Executes parallel extractions with live console feedback and diagnostic breakdown."""
    results: List[JobResult] = []
    args.snippets_dir.mkdir(parents=True, exist_ok=True)
    total = len(candidates)
    success = 0
    errors = 0
    err_breakdown: Dict[str, int] = {}

    with ThreadPoolExecutor(max_workers=args.workers) as executor:
        futures = {
            executor.submit(
                extract_media_snippet,
                job,
                args.snippets_dir,
                DEFAULT_DBSRT_BIN,
                DEFAULT_NAS_HOST,
                DEFAULT_NAS_USER,
                nas_pass,
                DEFAULT_FREEBOX_HTTP_PORT,
            ): job
            for job in candidates
        }

        for idx, fut in enumerate(as_completed(futures), 1):
            res = fut.result()
            results.append(res)
            st = res.status
            title = str(res.title)[:40]
            if st in ("EXTRACTED", "CACHED"):
                success += 1
                print(f"  [{idx:4d}/{total}] ✅ {st:<9} ({res.elapsed_sec:4.1f}s) : {title} ({res.source_type})")
            else:
                errors += 1
                err_breakdown[st] = err_breakdown.get(st, 0) + 1
                err = f": {res.error_message}" if res.error_message else ""
                print(f"  [{idx:4d}/{total}] ❌ {st:<9} ({res.elapsed_sec:4.1f}s) : {title}{err}")

    print("\n================================================================================")
    print(f" 🎉 EXTRACTION TERMINÉE : {success} succès, {errors} échecs / {total} traités.")
    if err_breakdown:
        print("  ⚠️ Répartition des échecs par statut :")
        for cat, cnt in sorted(err_breakdown.items()):
            print(f"    • {cat:<14}: {cnt} médias")
    print("================================================================================")
    return results


def run_pipeline(args: argparse.Namespace) -> int:
    """Orchestrates comprehensive media snippet inventory and extraction."""
    print("================================================================================")
    print(" 📥 INVENTAIRE & EXTRACTION EXHAUSTIVE NAS + FREEBOX (COUVERTURE 100%)")
    print("================================================================================")
    print(f"Catalogue binaire : {args.catalog}")
    print(f"Dossier snippets  : {args.snippets_dir}")
    print(f"Rapport de sortie : {args.report}")

    nas_pass = resolve_nas_password()
    if not nas_pass and not args.dry_run:
        print("❌ Impossible de résoudre le mot de passe NAS.")
        return 1

    catalog_medias = load_catalog_entries(args.catalog)
    storage_index = build_unified_storage_index()
    jobs = map_catalog_to_storage_jobs(catalog_medias, args.phash_dir, storage_index)

    _print_inventory_summary(jobs, args.snippets_dir)

    if args.dry_run:
        print("\n🔍 Mode DRY-RUN actif : Aucune extraction réseau effectuée.")
        return 0

    # Filter out already cached
    candidates: List[MediaJob] = []
    for j in jobs:
        safe_stem = sanitize_filename(Path(j.source_path or j.original_filename).stem)
        mp4 = args.snippets_dir / f"{safe_stem}_mid15s.mp4"
        if not (mp4.is_file() and mp4.stat().st_size > 1024) and j.source_type != "unlocated":
            candidates.append(j)

    if not candidates:
        print("✅ Tous les médias localisés sont déjà extraits et valides en local !")
        return 0

    to_process = candidates[: args.limit] if args.limit else candidates
    print(f"\n🚀 Lancement extraction de {len(to_process)} extraits ({args.workers} workers)...")
    results = _run_worker_pool(to_process, args, nas_pass)

    args.report.parent.mkdir(parents=True, exist_ok=True)
    with open(args.report, "w", encoding="utf-8") as fh:
        json.dump([asdict(r) for r in results], fh, indent=2, ensure_ascii=False)
    print(f"📄 Rapport détaillé enregistré : {args.report}")

    return 0


def main() -> int:
    return run_pipeline(parse_cli_args())


if __name__ == "__main__":
    raise SystemExit(main())
