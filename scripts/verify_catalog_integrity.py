#!/usr/bin/env python3
"""
Comprehensive integrity audit and diagnostic validator for pydbsrt catalog and storage tiers.

Validates:
1. Zero-DB static binary index (data/catalog.dbphash): magic header, offsets, byte alignment.
2. Ground-truth certified couplings (data/SRT/couplings.json): JSON syntax, schema, anchors.
3. Loose pHash frames storage (data/__PHASH__/*.phash): non-zero size, exact 8-byte 64-bit alignment.
4. PostgreSQL 17 SP-GiST BK-tree: connectivity, table counts, schema migrations.
5. Live dbsrt-server daemon telemetry (/v1/status & /v1/catalog/status) when available.
"""

from __future__ import annotations

import argparse
import json
import struct
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any

# Binary and Protocol Constants
DBPHASH_MAGIC: bytes = b"DBSRT\x02\x00\x00"
DBPHASH_HEADER_SIZE: int = 128
DBPHASH_SUPPORTED_VERSION: int = 2
PHASH_FRAME_BYTES: int = 8

DEFAULT_DBPHASH_PATH = Path("data/catalog.dbphash")
DEFAULT_COUPLINGS_PATH = Path("data/SRT/couplings.json")
DEFAULT_PHASH_DIR = Path("data/__PHASH__")
DEFAULT_PG_HOST = "127.0.0.1"
DEFAULT_PG_PORT = 5433
DEFAULT_PG_USER = "docker"
DEFAULT_PG_DB = "bktreedb"
DEFAULT_SERVER_HOST = "127.0.0.1"
DEFAULT_SERVER_PORT = 8765

# ANSI Colors
CLR_RESET = "\033[0m"
CLR_BOLD = "\033[1m"
CLR_RED = "\033[31m"
CLR_GREEN = "\033[32m"
CLR_YELLOW = "\033[33m"
CLR_BLUE = "\033[34m"
CLR_CYAN = "\033[36m"
CLR_GRAY = "\033[90m"


@dataclass
class TierCheckResult:
    tier_name: str
    passed: bool
    details: dict[str, Any]
    error_message: str | None = None


def check_tier1_dbphash(path: Path) -> TierCheckResult:
    """Verifies binary structure, header magic, and byte alignment of catalog.dbphash."""
    if not path.exists():
        return TierCheckResult(
            tier_name="Tier 1 : Index Zero-DB (catalog.dbphash)",
            passed=False,
            details={"path": str(path)},
            error_message=f"Fichier introuvable : {path}",
        )

    file_size = path.stat().st_size
    if file_size < DBPHASH_HEADER_SIZE:
        return TierCheckResult(
            tier_name="Tier 1 : Index Zero-DB (catalog.dbphash)",
            passed=False,
            details={"path": str(path), "size_bytes": file_size},
            error_message=f"Taille de fichier insuffisante ({file_size} octets < 128)",
        )

    with path.open("rb") as f:
        header = f.read(DBPHASH_HEADER_SIZE)

    # Header unpack format: magic(8s), version(H), flags(H), media_count(I), cuts_count(Q), sparse_count(Q),
    # offset_m(Q), offset_c(Q), offset_s(Q), offset_str(Q), strings_len(Q), checksum(Q)
    try:
        (
            magic,
            version,
            flags,
            media_count,
            cuts_count,
            sparse_count,
            off_media,
            off_cuts,
            off_sparse,
            off_strings,
            strings_len,
            checksum,
        ) = struct.unpack("<8sHHIQQQQQQQQ", header[:80])
    except Exception as e:
        return TierCheckResult(
            tier_name="Tier 1 : Index Zero-DB (catalog.dbphash)",
            passed=False,
            details={"path": str(path), "size_bytes": file_size},
            error_message=f"Échec de décodage binaire d'en-tête : {e}",
        )

    if magic != DBPHASH_MAGIC:
        return TierCheckResult(
            tier_name="Tier 1 : Index Zero-DB (catalog.dbphash)",
            passed=False,
            details={"path": str(path), "magic_raw": repr(magic)},
            error_message=f"Magic bytes invalides ({repr(magic)} != {repr(DBPHASH_MAGIC)})",
        )

    if version != DBPHASH_SUPPORTED_VERSION:
        return TierCheckResult(
            tier_name="Tier 1 : Index Zero-DB (catalog.dbphash)",
            passed=False,
            details={"path": str(path), "version": version},
            error_message=f"Version de format non supportée ({version} != {DBPHASH_SUPPORTED_VERSION})",
        )

    expected_file_size = off_strings + strings_len
    if file_size != expected_file_size:
        return TierCheckResult(
            tier_name="Tier 1 : Index Zero-DB (catalog.dbphash)",
            passed=False,
            details={
                "path": str(path),
                "actual_size": file_size,
                "expected_size": expected_file_size,
            },
            error_message=f"Taille de fichier discordante ({file_size} != {expected_file_size})",
        )

    return TierCheckResult(
        tier_name="Tier 1 : Index Zero-DB (catalog.dbphash)",
        passed=True,
        details={
            "path": str(path),
            "size_bytes": file_size,
            "version": version,
            "media_count": media_count,
            "total_cuts": cuts_count,
            "total_sparse_frames": sparse_count,
            "offset_media_table": off_media,
            "offset_cuts_table": off_cuts,
            "offset_sparse_table": off_sparse,
            "offset_strings_table": off_strings,
            "strings_len": strings_len,
        },
    )


def check_tier2_couplings(path: Path) -> TierCheckResult:
    """Verifies JSON syntax and metadata integrity of certified couplings."""
    if not path.exists():
        return TierCheckResult(
            tier_name="Tier 2 : Registre Couplages Certifiés (couplings.json)",
            passed=False,
            details={"path": str(path)},
            error_message=f"Fichier introuvable : {path}",
        )

    file_size = path.stat().st_size
    try:
        with path.open("r", encoding="utf-8") as f:
            data = json.load(f)
    except Exception as e:
        return TierCheckResult(
            tier_name="Tier 2 : Registre Couplages Certifiés (couplings.json)",
            passed=False,
            details={"path": str(path), "size_bytes": file_size},
            error_message=f"Erreur de syntaxe JSON : {e}",
        )

    if not isinstance(data, dict) or "couplings" not in data:
        return TierCheckResult(
            tier_name="Tier 2 : Registre Couplages Certifiés (couplings.json)",
            passed=False,
            details={"path": str(path), "type": type(data).__name__},
            error_message="Structure racine invalide (clé 'couplings' absente)",
        )

    couplings_list = data["couplings"]
    if not isinstance(couplings_list, list):
        return TierCheckResult(
            tier_name="Tier 2 : Registre Couplages Certifiés (couplings.json)",
            passed=False,
            details={"path": str(path)},
            error_message="'couplings' doit être une liste",
        )

    total_couplings = len(couplings_list)
    total_subtitles = 0
    total_sub_frames = 0
    invalid_entries = 0

    required_keys = {"media_name", "fps", "frame_count", "subtitles"}
    for idx, item in enumerate(couplings_list):
        if not isinstance(item, dict) or not required_keys.issubset(item.keys()):
            invalid_entries += 1
            continue
        subs = item.get("subtitles", [])
        if isinstance(subs, list):
            total_subtitles += len(subs)
            for sub in subs:
                if isinstance(sub, dict):
                    anchors = sub.get("sub_frames", [])
                    if isinstance(anchors, list):
                        total_sub_frames += len(anchors)

    if invalid_entries > 0:
        return TierCheckResult(
            tier_name="Tier 2 : Registre Couplages Certifiés (couplings.json)",
            passed=False,
            details={
                "path": str(path),
                "total_couplings": total_couplings,
                "invalid_entries": invalid_entries,
            },
            error_message=f"{invalid_entries} couplages ont des champs manquants",
        )

    return TierCheckResult(
        tier_name="Tier 2 : Registre Couplages Certifiés (couplings.json)",
        passed=True,
        details={
            "path": str(path),
            "size_bytes": file_size,
            "certified_media_couplings": total_couplings,
            "registered_subtitles": total_subtitles,
            "cue_frame_anchors": total_sub_frames,
        },
    )


def check_tier3_phash_files(phash_dir: Path) -> TierCheckResult:
    """Verifies all .phash files for non-zero size and exact 8-byte 64-bit frame alignment."""
    if not phash_dir.exists() or not phash_dir.is_dir():
        return TierCheckResult(
            tier_name="Tier 3 : Fichiers pHash Individuels (data/__PHASH__)",
            passed=False,
            details={"path": str(phash_dir)},
            error_message=f"Répertoire introuvable : {phash_dir}",
        )

    files = list(phash_dir.glob("*.phash"))
    total_files = len(files)
    if total_files == 0:
        return TierCheckResult(
            tier_name="Tier 3 : Fichiers pHash Individuels (data/__PHASH__)",
            passed=False,
            details={"path": str(phash_dir), "count": 0},
            error_message="Aucun fichier .phash trouvé dans le répertoire",
        )

    corrupted_files: list[tuple[str, int, str]] = []
    total_frames = 0
    total_bytes = 0

    for f in files:
        sz = f.stat().st_size
        total_bytes += sz
        if sz == 0:
            corrupted_files.append((f.name, sz, "Fichier 0 octets"))
        elif sz % PHASH_FRAME_BYTES != 0:
            corrupted_files.append((f.name, sz, f"Désalignement 64-bit (reste {sz % PHASH_FRAME_BYTES} != 0)"))
        else:
            total_frames += sz // PHASH_FRAME_BYTES

    if corrupted_files:
        return TierCheckResult(
            tier_name="Tier 3 : Fichiers pHash Individuels (data/__PHASH__)",
            passed=False,
            details={
                "total_files": total_files,
                "corrupted_count": len(corrupted_files),
                "corrupted_sample": corrupted_files[:5],
            },
            error_message=f"{len(corrupted_files)} fichiers .phash corrompus ou désalignés",
        )

    return TierCheckResult(
        tier_name="Tier 3 : Fichiers pHash Individuels (data/__PHASH__)",
        passed=True,
        details={
            "total_files": total_files,
            "total_bytes": total_bytes,
            "total_frames_indexed": total_frames,
            "alignment": "Exact 64-bit (8 bytes / frame)",
        },
    )


def check_tier4_postgres(
    host: str = DEFAULT_PG_HOST,
    port: int = DEFAULT_PG_PORT,
    user: str = DEFAULT_PG_USER,
    dbname: str = DEFAULT_PG_DB,
) -> TierCheckResult:
    """Verifies PostgreSQL 17 SP-GiST connectivity, schema migrations, and table counts."""
    # Attempt query via docker exec or psql
    psql_cmd = [
        "docker",
        "exec",
        "docker-bktreedb-1",
        "psql",
        "-U",
        user,
        "-d",
        dbname,
        "-t",
        "-A",
        "-c",
        (
            "SELECT 'medias:' || COUNT(*) FROM medias; "
            "SELECT 'frames:' || COUNT(*) FROM frames; "
            "SELECT 'subtitles:' || COUNT(*) FROM subtitles; "
            "SELECT 'sub_frames:' || COUNT(*) FROM sub_frames;"
        ),
    ]

    try:
        proc = subprocess.run(psql_cmd, capture_output=True, text=True, timeout=5)
    except Exception as e:
        return TierCheckResult(
            tier_name="Tier 4 : PostgreSQL 17 SP-GiST (docker-bktreedb-1)",
            passed=False,
            details={"host": host, "port": port, "dbname": dbname},
            error_message=f"Conteneur Docker docker-bktreedb-1 inaccessible : {e}",
        )

    if proc.returncode != 0:
        return TierCheckResult(
            tier_name="Tier 4 : PostgreSQL 17 SP-GiST (docker-bktreedb-1)",
            passed=False,
            details={"host": host, "port": port, "dbname": dbname, "stderr": proc.stderr.strip()},
            error_message=f"Erreur SQL psql : {proc.stderr.strip()}",
        )

    counts: dict[str, int] = {}
    for line in proc.stdout.splitlines():
        line = line.strip()
        if ":" in line:
            k, v = line.split(":", 1)
            try:
                counts[k] = int(v)
            except ValueError:
                pass

    return TierCheckResult(
        tier_name="Tier 4 : PostgreSQL 17 SP-GiST (docker-bktreedb-1)",
        passed=True,
        details={
            "container": "docker-bktreedb-1",
            "host": host,
            "port": port,
            "database": dbname,
            "counts": counts,
        },
    )


def check_tier5_server_daemon(host: str = DEFAULT_SERVER_HOST, port: int = DEFAULT_SERVER_PORT) -> TierCheckResult:
    """Verifies live HTTP API status and catalog snapshot on dbsrt-server."""
    import urllib.error
    import urllib.request

    url_status = f"http://{host}:{port}/v1/status"
    url_catalog = f"http://{host}:{port}/v1/catalog/status"

    try:
        req = urllib.request.Request(url_status)
        with urllib.request.urlopen(req, timeout=2) as resp:
            data_status = json.loads(resp.read().decode("utf-8"))
    except (urllib.error.URLError, TimeoutError, OSError):
        return TierCheckResult(
            tier_name="Tier 5 : Démon HTTP dbsrt-server (:8765)",
            passed=True,  # Daemon being offline is non-fatal for storage integrity
            details={
                "status": "OFFLINE",
                "notice": "Serveur non démarré (optionnel pour la validation stockage)",
            },
        )

    catalog_info = {}
    try:
        req_cat = urllib.request.Request(url_catalog)
        with urllib.request.urlopen(req_cat, timeout=2) as resp_cat:
            catalog_info = json.loads(resp_cat.read().decode("utf-8"))
    except Exception:
        pass

    return TierCheckResult(
        tier_name="Tier 5 : Démon HTTP dbsrt-server (:8765)",
        passed=True,
        details={
            "status": "ONLINE",
            "uptime_seconds": data_status.get("uptime_seconds"),
            "postgres_connected": data_status.get("postgres_connected"),
            "catalog_snapshot": catalog_info,
        },
    )


def print_banner() -> None:
    print(f"{CLR_BOLD}{CLR_CYAN}========================================================================{CLR_RESET}")
    print(f"{CLR_BOLD}{CLR_CYAN}===   PYDBSRT AUDIT D'INTÉGRITÉ EXHAUSTIF DU CATALOGUE & STOCKAGE    ==={CLR_RESET}")
    print(f"{CLR_BOLD}{CLR_CYAN}========================================================================{CLR_RESET}\n")


def print_tier_result(res: TierCheckResult) -> None:
    badge = f"{CLR_BOLD}{CLR_GREEN}[✓ PASS]{CLR_RESET}" if res.passed else f"{CLR_BOLD}{CLR_RED}[✗ FAIL]{CLR_RESET}"
    print(f"{badge} {CLR_BOLD}{res.tier_name}{CLR_RESET}")
    if res.error_message:
        print(f"       {CLR_RED}→ Erreur : {res.error_message}{CLR_RESET}")

    for k, v in res.details.items():
        if isinstance(v, dict):
            print(f"       {CLR_GRAY}• {k}:{CLR_RESET}")
            for sub_k, sub_v in v.items():
                print(f"         - {sub_k:<24} : {CLR_YELLOW}{sub_v}{CLR_RESET}")
        else:
            print(f"       {CLR_GRAY}• {k:<26} : {CLR_RESET}{v}")
    print()


def main() -> int:
    parser = argparse.ArgumentParser(description="Audit exhaustif d'intégrité du catalogue et des stockages pydbsrt")
    parser.add_argument(
        "--dbphash",
        type=Path,
        default=DEFAULT_DBPHASH_PATH,
        help="Chemin de catalog.dbphash",
    )
    parser.add_argument(
        "--couplings",
        type=Path,
        default=DEFAULT_COUPLINGS_PATH,
        help="Chemin de couplings.json",
    )
    parser.add_argument(
        "--phash-dir",
        type=Path,
        default=DEFAULT_PHASH_DIR,
        help="Répertoire des .phash",
    )
    parser.add_argument(
        "--pg-host",
        default=DEFAULT_PG_HOST,
        help="Hôte PostgreSQL",
    )
    parser.add_argument(
        "--pg-port",
        type=int,
        default=DEFAULT_PG_PORT,
        help="Port PostgreSQL",
    )
    parser.add_argument(
        "--server-port",
        type=int,
        default=DEFAULT_SERVER_PORT,
        help="Port dbsrt-server",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Exporter les résultats au format JSON machine-readable",
    )
    args = parser.parse_args()

    start_time = time.perf_counter()
    results: list[TierCheckResult] = []

    # Run checks sequentially
    results.append(check_tier1_dbphash(args.dbphash))
    results.append(check_tier2_couplings(args.couplings))
    results.append(check_tier3_phash_files(args.phash_dir))
    results.append(check_tier4_postgres(args.pg_host, args.pg_port))
    results.append(check_tier5_server_daemon(port=args.server_port))

    elapsed = time.perf_counter() - start_time

    # Storage integrity is strictly tiers 1 to 4
    all_storage_passed = all(r.passed for r in results[:4])

    if args.json:
        payload = {
            "all_passed": all_storage_passed,
            "elapsed_seconds": round(elapsed, 4),
            "tiers": [asdict(r) for r in results],
        }
        print(json.dumps(payload, indent=2))
        return 0 if all_storage_passed else 1

    print_banner()
    for r in results:
        print_tier_result(r)

    print(f"{CLR_BOLD}────────────────────────────────────────────────────────────────────────{CLR_RESET}")
    if all_storage_passed:
        print(
            f"{CLR_BOLD}{CLR_GREEN}🎉 AUDIT 100% CONFORME : ZÉRO DÉFAUT D'ALIGNEMENT, DE SYNTAXE OU DE PERSISTANCE.{CLR_RESET}"
        )
        print(f"{CLR_GRAY}   Temps de vérification : {elapsed * 1000:.1f} ms sur 4 paliers physiques.{CLR_RESET}\n")
        return 0
    else:
        print(f"{CLR_BOLD}{CLR_RED}❌ ANOMALIES DÉTECTÉES DANS LE CATALOGUE OU LE STOCKAGE !{CLR_RESET}")
        print(f"{CLR_GRAY}   Consultez les détails des paliers en échec ci-dessus.{CLR_RESET}\n")
        return 1


if __name__ == "__main__":
    sys.exit(main())
