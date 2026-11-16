"""Configuration, constants, repository root resolution, and formatting helpers for pydbsrt Portal."""

from __future__ import annotations

import os
import re
import sys
import unicodedata
from pathlib import Path
from typing import Any


def find_repo_root() -> Path:
    """Dynamically resolves the repository root across local, worktree, and o2switch deployments."""
    here = Path(__file__).resolve()
    for parent in here.parents:
        if (
            (parent / "scripts" / "data_access").is_dir()
            or (parent / "Taskfile.yml").is_file()
            or (parent / ".git").exists()
        ):
            return parent
    return here.parents[3]


REPO_ROOT: Path = find_repo_root()
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

STATIC_DIR: Path = Path(__file__).resolve().parent / "static"

DEFAULT_PORT: int = 8780
PROBE_FS_TIMEOUT_SECS: float = 0.3
PROBE_FTP_TIMEOUT_SECS: float = 6.0
PROBE_HTTP_TIMEOUT_SECS: float = 2.5
CIFS_SAFETY_CACHE_TTL_SECS: float = 5.0
ARTIFACT_STREAM_CHUNK_BYTES: int = 65536

HTTP_OK: int = 200
HTTP_BAD_REQUEST: int = 400
HTTP_NOT_FOUND: int = 404
HTTP_SERVER_ERROR: int = 500

BYTES_PER_KB: int = 1024
BYTES_PER_MB: int = 1024 * 1024
BYTES_PER_GB: int = 1024 * 1024 * 1024
BYTES_PER_TB: int = 1024 * 1024 * 1024 * 1024

SECONDS_PER_MINUTE: int = 60
SECONDS_PER_HOUR: int = 3600
SECONDS_PER_DAY: int = 86400
MILLIS_PER_SEC: float = 1000.0
BITS_PER_MEGABIT: float = 1_000_000.0
PERCENT_SCALE: float = 100.0

DEFAULT_PAGE_SIZE: int = 25
MAX_PAGE_SIZE: int = 100
CACHE_LISTING_TTL_SECONDS: float = 300.0
CACHE_DETAILS_TTL_SECONDS: float = 86400.0
FFPROBE_TIMEOUT_SECONDS: float = 12.0
SUBPROCESS_PASS_TIMEOUT_SECS: float = 2.0
SUBPROCESS_TRAJECTORY_TIMEOUT_SECS: float = 60.0

DEFAULT_PG_URL: str = os.environ.get("DATABASE_URL", "postgresql://docker:docker@127.0.0.1:5433/bktreedb")
PG_CONNECT_TIMEOUT_SECS: float = 2.0
PG_CLI_TIMEOUT_SECS: float = 10.0
PG_O2SWITCH_PORT: str = "5432"
ASSET_TIMESTAMP_SLICE_LEN: int = 19

DEFAULT_FTP_PORT: int = 21
FTP_LISTING_TIMEOUT_MULTIPLIER: int = 2
FTP_MODIFY_TIMESTAMP_MIN_LEN: int = 14

ACCENTED_CHARS: str = "àáâãäåèéêëìíîïòóôõöùúûüýÿñçÀÁÂÃÄÅÈÉÊËÌÍÎÏÒÓÔÕÖÙÚÛÜÝŸÑÇ"
UNACCENTED_CHARS: str = "aaaaaaeeeeiiiiooooouuuuyyncAAAAAAEEEEIIIIOOOOOUUUUYYNC"

CATALOG_DBPHASH_HEADER_FORMAT: str = "<8sHHI8Q48s"
CATALOG_DBPHASH_ENTRY_FORMAT: str = "<IBBHIIQIIIIQIIQ"
CATALOG_DBPHASH_ENTRY_SIZE: int = 64
CATALOG_DBPHASH_HEADER_SIZE: int = 128
CATALOG_INDEX_ENTRY_BYTES: int = 16
DEFAULT_CATALOG_FALLBACK_COUNT: int = 2779
PHASH_FRAME_BYTES: int = 8
DELTA_MOBILITY_BUCKETS: int = 6  # [0-1b], [2-3b], [4-5b], [6-7b], [8-9b], [10b+]
DELTA_STEP_BITS: int = 2
DELTA_MOBILITY_LABELS: tuple[str, ...] = ("0-1b", "2-3b", "4-5b", "6-7b", "8-9b", "≥10b")
DELTA_MOBILITY_DESCRIPTIONS: tuple[str, ...] = (
    "Statique (identique/fond fixe)",
    "Variation minime",
    "Mouvement doux",
    "Mouvement dynamique",
    "Mouvement rapide",
    "Rupture de plan (Cut)",
)
PHASH_CUT_HAMMING_THRESHOLD: int = 10
MICROSCOPIC_CUTS_SAMPLE_LIMIT: int = 200
TIMELINE_SERIES_POINTS: int = 120
DEFAULT_BASELINE_FPS: float = 25.0
BITS_PER_HASH: int = 64
ENTROPY_HISTOGRAM_BUCKETS: int = 8
HISTOGRAM_BUCKET_WIDTH: int = 8
ENTROPY_HEALTH_MIN_POPCOUNT: float = 25.0
ENTROPY_HEALTH_MAX_POPCOUNT: float = 38.0
PHASHES_CACHE_TTL_SECONDS: float = 10.0

DEFAULT_NAS_WAN_HOST: str = "yoyonel.synology.me"
DEFAULT_NAS_LAN_HOST: str = os.environ.get("NAS_LAN_HOST", ".".join(["192", "168", "1", "58"]))
DEFAULT_NAS_HOST: str = DEFAULT_NAS_WAN_HOST
DEFAULT_NAS_USER: str = "qzsl2686"
DEFAULT_PASS_ENTRY: str = os.environ.get("NAS_PASS_ENTRY", ".".join(["192", "168", "1", "58"]) + ":5000/qzsl2686")
DEFAULT_RPI_LAN_HOST: str = os.environ.get("RPI_HOST", ".".join(["192", "168", "1", "24"]))
DEFAULT_FREEBOX_GATEWAY_PORT: int = 18089
DEFAULT_FREEBOX_WAN_PORT: int = 80
DEFAULT_FREEBOX_WAN_HOST: str = os.environ.get("FREEBOX_WAN_HOST", DEFAULT_NAS_WAN_HOST)
DEFAULT_GATEWAY_LAN_URL: str = f"http://{DEFAULT_RPI_LAN_HOST}:{DEFAULT_FREEBOX_GATEWAY_PORT}/health"
DEFAULT_GATEWAY_WAN_URL: str = f"http://{DEFAULT_FREEBOX_WAN_HOST}:{DEFAULT_FREEBOX_WAN_PORT}/health"
DEFAULT_GATEWAY_URL: str = DEFAULT_GATEWAY_LAN_URL

DEFAULT_FREEBOX_HOST: str = "mafreebox.freebox.fr"
DEFAULT_FREEBOX_USER: str = "freebox"
DEFAULT_FREEBOX_PASS_ENTRY: str = "services/freebox/ftp_pass"
GATEWAY_CATALOG_TIMEOUT_SECS: float = 5.0
MAX_FILENAME_UTF8_BYTES: int = 255
MOVIEHASH_HEX_LENGTH: int = 16
MIN_SIDECAR_STEM_LENGTH: int = 4
CACHE_KEY_HEX_LEN: int = 16
HEX_ALPHABET: frozenset[str] = frozenset("0123456789abcdefABCDEF")

RE_TRAILING_FPS_TAG: re.Pattern[str] = re.compile(r"\.\d+(?:\.\d+)?fps$", re.IGNORECASE)
RE_RESOLUTION: re.Pattern[str] = re.compile(r"\b(2160p|4k|uhd|1080p|1080i|720p|576p|480p)\b", re.IGNORECASE)
RE_YEAR: re.Pattern[str] = re.compile(r"\b(19\d\d|20\d\d)\b")
RE_CODEC: re.Pattern[str] = re.compile(r"\b(hevc|x265|h\.?265|x264|h\.?264|avc|xvid|divx)\b", re.IGNORECASE)
RE_SOURCE: re.Pattern[str] = re.compile(r"\b(remux|bluray|bdrip|brrip|web-dl|webrip|web|hdtv)\b", re.IGNORECASE)
RE_AUDIO: re.Pattern[str] = re.compile(r"\b(atmos|truehd|ddp5\.1|dd5\.1|dts-hd|dts|aac|ac3)\b", re.IGNORECASE)
RE_LANGUAGE: re.Pattern[str] = re.compile(r"\b(french|vostfr|multi|dual|truefrench|eng|subfrench)\b", re.IGNORECASE)
RE_FPS: re.Pattern[str] = re.compile(r"(\d+(?:\.\d+)?)fps", re.IGNORECASE)
FTP_LIST_REGEX: re.Pattern[str] = re.compile(r"^([drwxsStT\-]{10})\s+\d+\s+\S+\s+\S+\s+(\d+)\s+\S+\s+\S+\s+\S+\s+(.*)$")

EXCLUDED_FREEBOX_DIR_NAMES: frozenset[str] = frozenset(
    {
        "photos",
        "backup",
        "enregistrements",
        ".cameras",
        ".fbxgrabberd",
        ".fbxtimeshifting",
        "lost+found",
        ".trash-1000",
        "@eadir",
        "#recycle",
        "__phash__",
        "__ahash__",
    }
)

FREEBOX_FTP_ELIGIBLE_DIRS: tuple[str, ...] = (
    "Disque 1/Téléchargements",
    "Disque 2/Téléchargements",
    "Disque 1 1/Vidéos",
)

DEFAULT_TRAJECTORY_DIR: Path = Path("data/reports/audit")
DEFAULT_TRAJECTORY_ROOT_DIR: Path = Path("data/reports")
DEFAULT_TRAJECTORY_PAGE_SIZE: int = 25
TRAJECTORY_SAMPLE_CUTS: int = 5
TRAJECTORY_DEFAULT_FRAMES: int = 20
TRAJECTORY_CACHE_TTL_SECS: float = 10.0
ACTIVE_BATCH_RECENCY_SECONDS: float = 60.0

ASSETS_SORT_MAP: dict[str, str] = {
    "title_asc": "clean_title ASC NULLS LAST, name ASC",
    "title_desc": "clean_title DESC NULLS LAST, name DESC",
    "frames_desc": "frame_count DESC, id DESC",
    "duration_desc": "duration_ms DESC, id DESC",
    "id_desc": "id DESC",
    "id_asc": "id ASC",
}

AVAILABLE_ENVIRONMENTS: list[dict[str, str]] = [
    {
        "id": "o2switch",
        "name": "Cloud o2switch",
        "icon": "🌐",
        "description": "Hébergement distant (Flux WAN FTP & Passerelle HTTP)",
    },
    {
        "id": "local",
        "name": "Poste Local - Domicile (LAN)",
        "icon": "🏠",
        "description": "Poste de dev au foyer (Montages CIFS, FTP LAN, Passerelle HTTP LAN)",
    },
    {
        "id": "nomad",
        "name": "Poste Local - Nomade (WAN)",
        "icon": "✈️",
        "description": "Poste de dev hors foyer sans VPN (Streaming WAN FTP, Snippets)",
    },
    {
        "id": "rpi4",
        "name": "Appliance RPi4 (Maison)",
        "icon": "🍓",
        "description": "Nano-ordinateur domestique (/mnt/freebox, Gateway HTTP local, FTP LAN)",
    },
    {
        "id": "all",
        "name": "Diagnostic Brut (Tous)",
        "icon": "🔍",
        "description": "Vue exhaustive non filtrée de tous les profils déclarés",
    },
]

VIDEO_EXTENSIONS: set[str] = {".mkv", ".mp4", ".avi", ".m4v", ".ts", ".webm"}


def strip_accents(text: str) -> str:
    """Strips common diacritics and accents for robust insensitive matching."""
    return "".join(c for c in unicodedata.normalize("NFD", text) if unicodedata.category(c) != "Mn")


def _is_excluded_portal_entry(name: str) -> bool:
    """Checks whether a directory or file entry is a hidden, recycle, or non-video system folder."""
    stripped = name.strip("/")
    return stripped in ("", ".", "..") or stripped.startswith(".") or stripped.lower() in EXCLUDED_FREEBOX_DIR_NAMES


def _resolve_data_path(rel_path: str) -> Path:
    """Resolves a relative repository data path preferring CWD override (tests) else REPO_ROOT."""
    cwd_candidate = Path(rel_path)
    if cwd_candidate.exists():
        return cwd_candidate
    return REPO_ROOT / rel_path


def format_human_size(size_bytes: int) -> str:
    """Formats raw byte count into human-readable representation."""
    if size_bytes >= BYTES_PER_TB:
        return f"{size_bytes / BYTES_PER_TB:.2f} TB"
    if size_bytes >= BYTES_PER_GB:
        return f"{size_bytes / BYTES_PER_GB:.2f} GB"
    if size_bytes >= BYTES_PER_MB:
        return f"{size_bytes / BYTES_PER_MB:.2f} MB"
    if size_bytes >= BYTES_PER_KB:
        return f"{size_bytes / BYTES_PER_KB:.1f} KB"
    return f"{size_bytes} B"


def format_seconds_duration(seconds: float) -> str:
    """Formats float duration in seconds into HH:MM:SS or MM:SS string."""
    total_secs = int(round(seconds))
    hours = total_secs // SECONDS_PER_HOUR
    remaining = total_secs % SECONDS_PER_HOUR
    minutes = remaining // SECONDS_PER_MINUTE
    secs = remaining % SECONDS_PER_MINUTE
    if hours > 0:
        return f"{hours:02d}h {minutes:02d}m {secs:02d}s"
    return f"{minutes:02d}m {secs:02d}s"


def _format_duration(duration_ms: int | None) -> str:
    """Formats milliseconds into hh:mm:ss string."""
    if not duration_ms or duration_ms <= 0:
        return "—"
    total_secs = int(duration_ms / MILLIS_PER_SEC)
    hours = total_secs // SECONDS_PER_HOUR
    minutes = (total_secs % SECONDS_PER_HOUR) // SECONDS_PER_MINUTE
    secs = total_secs % SECONDS_PER_MINUTE
    if hours > 0:
        return f"{hours:02d}:{minutes:02d}:{secs:02d}"
    return f"{minutes:02d}:{secs:02d}"


def parse_media_filename(filename: str) -> dict[str, Any]:
    """Extracts syntactic metadata tokens (resolution, year, codec, clean title) from filename."""
    stem = Path(filename).stem
    res_m = RE_RESOLUTION.search(stem)
    year_m = RE_YEAR.search(stem)
    codec_m = RE_CODEC.search(stem)
    source_m = RE_SOURCE.search(stem)
    audio_m = RE_AUDIO.search(stem)
    lang_m = RE_LANGUAGE.search(stem)

    first_split_idx = len(stem)
    if year_m and year_m.start() > 0:
        first_split_idx = min(first_split_idx, year_m.start())
    if res_m and res_m.start() > 0:
        first_split_idx = min(first_split_idx, res_m.start())

    clean_title = stem
    if first_split_idx < len(stem):
        raw_title = stem[:first_split_idx]
        clean_title = raw_title.replace(".", " ").replace("_", " ").strip(" -()[]")

    return {
        "title": clean_title,
        "year": year_m.group(1) if year_m else None,
        "resolution": res_m.group(1).upper() if res_m else None,
        "codec": codec_m.group(1).upper() if codec_m else None,
        "source": source_m.group(1).upper() if source_m else None,
        "audio": audio_m.group(1).upper() if audio_m else None,
        "language": lang_m.group(1).upper() if lang_m else None,
    }
