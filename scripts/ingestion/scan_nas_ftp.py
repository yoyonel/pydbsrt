#!/usr/bin/env python3
"""Robust, resilient Synology NAS FTP scanner with explicit timeout and error tracking.

Walks NAS directories (tvshow, video) using CWD + MLSD to prevent silent skips on
folders with special characters, spaces, or emojis. Tracks all network exceptions,
timeouts, and permission errors with structured reporting and retry mechanisms.
"""

from __future__ import annotations

import argparse
import ftplib
import json
import os
import subprocess
import sys
import time
import unicodedata
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Dict, List, Tuple

DEFAULT_NAS_HOST = os.environ.get("NAS_HOST", "yoyonel.synology.me")
DEFAULT_NAS_USER = os.environ.get("NAS_USER", "qzsl2686")
DEFAULT_PASS_ENTRY = os.environ.get("NAS_PASS_ENTRY", "nas/qzsl2686")
DEFAULT_FTP_PORT = 21
DEFAULT_SOCKET_TIMEOUT = 20.0
DEFAULT_MAX_RETRIES = 3
DEFAULT_RETRY_DELAY = 2.0
DEFAULT_OUT_REPORT = Path("data/reports/nas_ftp_index.json")

VIDEO_EXTENSIONS = (".mkv", ".mp4", ".avi", ".webm", ".mov", ".m4v")
IGNORED_DIR_NAMES = {".", "..", "@eaDir", ".@__thumb", ".Trash-1000", "#recycle"}


@dataclass
class ScanError:
    """Explicit trace of a directory listing failure or timeout."""

    path: str
    attempt: int
    error_type: str
    error_message: str
    elapsed_sec: float


@dataclass
class ScanInventory:
    """Comprehensive inventory and health metrics of NAS FTP scan."""

    scanned_at: str
    elapsed_sec: float
    total_dirs_scanned: int
    total_videos_found: int
    total_errors: int
    errors: List[ScanError] = field(default_factory=list)
    index: Dict[str, str] = field(default_factory=dict)


def resolve_nas_password(pass_entry: str = DEFAULT_PASS_ENTRY) -> str:
    """Resolves NAS password via pass store, o2switch fallback, or environment variable."""
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


class ResilientFtpClient:
    """Manages an auto-reconnecting, timeout-bounded FTP connection."""

    def __init__(
        self,
        host: str,
        user: str,
        password: str,
        port: int = DEFAULT_FTP_PORT,
        timeout: float = DEFAULT_SOCKET_TIMEOUT,
    ):
        self.host = host
        self.user = user
        self.password = password
        self.port = port
        self.timeout = timeout
        self.ftp: ftplib.FTP | None = None

    def connect(self) -> None:
        """Establishes a new authenticated FTP connection."""
        self.close()
        ftp = ftplib.FTP()
        ftp.connect(self.host, self.port, timeout=self.timeout)
        ftp.login(self.user, self.password)
        ftp.encoding = "utf-8"
        self.ftp = ftp

    def close(self) -> None:
        """Safely closes existing connection without raising errors."""
        if self.ftp is not None:
            try:
                self.ftp.quit()
            except (OSError, ftplib.Error):
                try:
                    self.ftp.close()
                except OSError:
                    pass
            self.ftp = None

    def ensure_connected(self) -> ftplib.FTP:
        """Returns active connection, reconnecting if absent."""
        if self.ftp is None:
            self.connect()
        assert self.ftp is not None
        return self.ftp

    def list_dir_cwd(self, dir_path: str) -> List[Tuple[str, Dict[str, str]]]:
        """Lists directory entries via CWD + MLSD for strict Synology compatibility."""
        ftp = self.ensure_connected()
        ftp.cwd(dir_path)
        return list(ftp.mlsd())


def list_directory_with_retry(
    client: ResilientFtpClient,
    dir_path: str,
    max_retries: int = DEFAULT_MAX_RETRIES,
    retry_delay: float = DEFAULT_RETRY_DELAY,
) -> Tuple[List[Tuple[str, Dict[str, str]]], ScanError | None]:
    """Attempts CWD + MLSD directory listing with explicit retries, logging, and error tracing."""
    last_err: ScanError | None = None

    for attempt in range(1, max_retries + 1):
        t0 = time.monotonic()
        try:
            entries = client.list_dir_cwd(dir_path)
            return entries, None
        except (
            TimeoutError,
            ftplib.error_temp,
            ftplib.error_proto,
            ftplib.error_reply,
            EOFError,
            ConnectionResetError,
            BrokenPipeError,
            OSError,
        ) as exc:
            elapsed = round(time.monotonic() - t0, 2)
            err_type = type(exc).__name__
            err_msg = str(exc) or "unknown network failure"
            last_err = ScanError(dir_path, attempt, err_type, err_msg, elapsed)

            print(
                f"  ⚠️ [FTP RETRY {attempt}/{max_retries}] ({elapsed}s) {dir_path} -> {err_type}: {err_msg}",
                file=sys.stderr,
            )
            client.close()
            time.sleep(retry_delay * attempt)
        except ftplib.error_perm as exc:
            elapsed = round(time.monotonic() - t0, 2)
            last_err = ScanError(dir_path, attempt, "PermissionDenied", str(exc), elapsed)
            print(f"  ❌ [FTP PERM ERROR] ({elapsed}s) {dir_path} -> {exc}", file=sys.stderr)
            return [], last_err

    return [], last_err


def _process_dir_entries(
    curr_dir: str,
    entries: List[Tuple[str, Dict[str, str]]],
    stack: List[str],
    index: Dict[str, str],
) -> int:
    """Classifies directory entries into subdirectories and video files."""
    new_videos = 0
    for name, facts in entries:
        if name in IGNORED_DIR_NAMES or name.startswith("."):
            continue

        full_path = f"{curr_dir}/{name}" if curr_dir != "/" else f"/{name}"
        ftype = facts.get("type", "")

        if ftype == "dir":
            stack.append(full_path)
        elif ftype == "file" and name.lower().endswith(VIDEO_EXTENSIONS):
            norm_name = unicodedata.normalize("NFC", name)
            clean_rel = full_path.lstrip("/")
            index[norm_name] = clean_rel
            new_videos += 1

    return new_videos


def walk_nas_root(
    client: ResilientFtpClient,
    root_folder: str,
    index: Dict[str, str],
    errors: List[ScanError],
) -> Tuple[int, int]:
    """Exhaustively walks a root NAS folder using CWD + MLSD stack navigation."""
    norm_root = f"/{root_folder.strip('/')}"
    stack = [norm_root]
    dirs_count = 0
    videos_count = 0

    print(f"📂 Lancement du scan récursif FTP sur '{norm_root}'...")
    while stack:
        curr = stack.pop()
        dirs_count += 1

        entries, err = list_directory_with_retry(client, curr)
        if err is not None:
            errors.append(err)
            continue

        found = _process_dir_entries(curr, entries, stack, index)
        videos_count += found

        if dirs_count % 50 == 0:
            print(f"  ... exploré {dirs_count} dossiers, {videos_count} vidéos indexées ({len(errors)} erreurs)")

    return dirs_count, videos_count


def parse_arguments() -> argparse.Namespace:
    """Parses command-line arguments."""
    parser = argparse.ArgumentParser(description="Robust Synology NAS FTP scanner with explicit error handling.")
    parser.add_argument("--host", default=DEFAULT_NAS_HOST, help="NAS host (default: yoyonel.synology.me)")
    parser.add_argument("--user", default=DEFAULT_NAS_USER, help="NAS FTP username (default: qzsl2686)")
    parser.add_argument("--port", type=int, default=DEFAULT_FTP_PORT, help="NAS FTP port (default: 21)")
    parser.add_argument("--timeout", type=float, default=DEFAULT_SOCKET_TIMEOUT, help="Socket timeout in seconds")
    parser.add_argument("--retries", type=int, default=DEFAULT_MAX_RETRIES, help="Max retries per directory")
    parser.add_argument("--roots", nargs="+", default=["tvshow", "video"], help="Root folders to scan")
    parser.add_argument("--output", type=Path, default=DEFAULT_OUT_REPORT, help="Output JSON report path")
    return parser.parse_args()


def run_scanner(args: argparse.Namespace) -> int:
    """Orchestrates comprehensive FTP crawl with full error and timeout diagnostics."""
    t0 = time.monotonic()
    print("================================================================================")
    print(" 📡 SCANNER NAS FTP SYNOLOGY (CWD + MLSD AVEC GESTION COMPLÈTE DES ERREURS)")
    print("================================================================================")
    print(f"Hôte NAS    : {args.host}:{args.port}")
    print(f"Utilisateur : {args.user}")
    print(f"Dossiers    : {', '.join(args.roots)}")
    print(f"Rapport     : {args.output}")

    nas_pass = resolve_nas_password()
    if not nas_pass:
        print("❌ ERREUR FATALE: Impossible de résoudre le mot de passe NAS (pass store & o2switch vides).")
        return 1

    client = ResilientFtpClient(args.host, args.user, nas_pass, args.port, args.timeout)
    index: Dict[str, str] = {}
    errors: List[ScanError] = []
    total_dirs = 0
    total_videos = 0

    try:
        client.connect()
        print("✓ Connexion FTP authentifiée avec succès.")
    except Exception as exc:
        print(f"❌ ERREUR FATALE lors de la connexion initiale: {type(exc).__name__}: {exc}")
        return 1

    try:
        for root in args.roots:
            d_cnt, v_cnt = walk_nas_root(client, root, index, errors)
            total_dirs += d_cnt
            total_videos += v_cnt
    finally:
        client.close()

    elapsed = round(time.monotonic() - t0, 2)
    inv = ScanInventory(
        scanned_at=time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        elapsed_sec=elapsed,
        total_dirs_scanned=total_dirs,
        total_videos_found=total_videos,
        total_errors=len(errors),
        errors=errors,
        index=index,
    )

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with open(args.output, "w", encoding="utf-8") as fh:
        json.dump(asdict(inv), fh, indent=2, ensure_ascii=False)

    print("\n================================================================================")
    print(" 📊 BILAN COMPLET DU SCAN FTP NAS")
    print("================================================================================")
    print(f"  • Durée totale         : {elapsed}s")
    print(f"  • Dossiers explorés    : {total_dirs}")
    print(f"  • Vidéos indexées      : {total_videos}")
    print(f"  • Erreurs & Timeouts   : {len(errors)}")

    if errors:
        print("\n  ⚠️ DÉTAIL DES ÉCHECS ET TIMEOUTS :")
        for err in errors:
            print(
                f"    - [{err.error_type}] {err.path} (tentative {err.attempt}, {err.elapsed_sec}s): {err.error_message}"
            )
    else:
        print("  ✅ Aucune erreur ni aucun timeout réseau non résolu !")

    print(f"\n📄 Index persistant enregistré : {args.output}")
    print("================================================================================")
    return 0


def main() -> int:
    return run_scanner(parse_arguments())


if __name__ == "__main__":
    raise SystemExit(main())
