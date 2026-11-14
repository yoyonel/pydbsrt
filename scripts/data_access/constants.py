"""Named constants, timeouts, ports, and domain invariants for pydbsrt data access."""

from __future__ import annotations

# --- CLI Exit Codes ---
EXIT_OK: int = 0
EXIT_USAGE_ERROR: int = 2
EXIT_UNAVAILABLE: int = 3
EXIT_STALE_OR_DRIFT: int = 4

# --- Desynchronization Risk Tiers ---
TIER_IMMUTABLE_UNIT: str = "immutable_unit"
TIER_EVOLVING_CORPUS: str = "evolving_corpus"
TIER_LIVE_STATE: str = "live_state"

# --- Host & Network Situation Identifiers ---
NET_HOME_LAN: str = "home_lan"
NET_WORK_VPN: str = "work_vpn"
NET_NOMAD_WAN: str = "nomad_wan"
NET_O2SWITCH: str = "o2switch"
NET_RPI4: str = "rpi4"
VALID_SIMULATED_PROFILES: tuple[str, ...] = (
    NET_HOME_LAN,
    NET_WORK_VPN,
    NET_NOMAD_WAN,
    NET_O2SWITCH,
    NET_RPI4,
)

# --- Kernel Topology Inspection Constants ---
HOME_SUBNET_CIDR: str = ".".join(("192", "168", "1", "0")) + "/24"
CORP_VPN_HIJACK_CIDR: str = ".".join(("192", "168", "0", "0")) + "/16"
TUNNEL_DEVICE_PREFIXES: tuple[str, ...] = ("ppp", "tun", "wg", "tap")
IGNORED_IFACE_PREFIXES: tuple[str, ...] = ("lo", "br-", "docker", "virbr", "veth")
IGNORED_NM_TYPES: tuple[str, ...] = ("bridge", "loopback")
HOME_WIFI_SSID_PREFIX: str = "Freebox"
CORP_WIFI_SSID_PREFIX: str = "Letsignit"
PROC_MOUNTS_PATH: str = "/proc/mounts"
CIFS_FSTYPE: str = "cifs"
NMCLI_MIN_FIELDS: int = 3
PROC_MOUNT_MIN_FIELDS: int = 3

# --- Default Network Endpoints (Overridable via .env / Environment) ---
DEFAULT_NAS_LAN_IP: str = ".".join(("192", "168", "1", "58"))
DEFAULT_RPI4_LAN_IP: str = ".".join(("192", "168", "1", "24"))
DEFAULT_FREEBOX_LAN_IP: str = ".".join(("192", "168", "1", "254"))
DEFAULT_FREEBOX_HOSTNAME: str = "mafreebox.freebox.fr"
DEFAULT_FREEBOX_USER: str = "freebox"
DEFAULT_LOCALHOST_IP: str = ".".join(("127", "0", "0", "1"))
SSH_ALIAS_RPI4: str = "rpi4"
SSH_ALIAS_O2SWITCH: str = "o2switch"
REMOTE_DBSRT_DATA_REL: str = "dbsrt/data"

# --- Standard Service Ports ---
FTP_DEFAULT_PORT: int = 21
SSH_DEFAULT_PORT: int = 22
HTTP_DEFAULT_PORT: int = 80
HTTPS_DEFAULT_PORT: int = 443
FREEBOX_GATEWAY_PORT: int = 18089
PG_LOCAL_PORT: int = 5433
PG_RPI4_PORT: int = 5432
DBSRT_SERVER_PORT: int = 8765
HTTP_STATUS_OK: int = 200

# --- Bounded Probe & Command Timeouts (Seconds) ---
NMCLI_TIMEOUT_SECS: float = 1.5
IP_CMD_TIMEOUT_SECS: float = 1.0
GIT_CMD_TIMEOUT_SECS: float = 1.5
PASS_LOOKUP_TIMEOUT_SECS: float = 2.5
TCP_SOCKET_TIMEOUT_SECS: float = 1.2
HTTP_GATEWAY_TIMEOUT_SECS: float = 2.5
HTTPS_API_TIMEOUT_SECS: float = 4.5
FTP_PROBE_TIMEOUT_SECS: float = 4.0
SSH_CONNECT_TIMEOUT_SECS: int = 3
SSH_PROBE_TIMEOUT_SECS: float = 4.5
FETCH_CMD_TIMEOUT_SECS: float = 120.0

# --- Cache & Freshness Thresholds ---
CACHE_SCHEMA_VERSION: int = 2
CACHE_TTL_SECONDS: float = 300.0
CACHE_MAX_STALE_SECONDS: float = 86400.0
STALE_MTIME_TOLERANCE_SECONDS: float = 60.0
CACHE_DIR_REL: tuple[str, ...] = (".cache", "dbsrt")
CACHE_FILENAME: str = "data_access_cache.json"
CACHE_LOCK_FILENAME: str = "data_access.lock"
COUPLINGS_INDEX_CACHE_FILENAME: str = "couplings_index_v1.json"
SSH_CONTROL_PERSIST_SECS: int = 600
CACHE_DIR_MODE: int = 0o700
CACHE_FILE_MODE: int = 0o600
MAX_PROBE_WORKERS: int = 8
HTTP_READ_MAX_BYTES: int = 8192
MAX_RESOLVE_MATCHES: int = 25

# --- Unit Conversion Factors ---
MILLIS_PER_SEC: float = 1000.0
BYTES_PER_KB: int = 1024
BYTES_PER_MB: int = 1024 * 1024
BYTES_PER_GB: int = 1024 * 1024 * 1024
SECONDS_PER_MINUTE: int = 60
SECONDS_PER_HOUR: int = 3600
SECONDS_PER_DAY: int = 86400

# --- Binary Format & Dataset Constants ---
DBPHASH_HEADER_PREFIX_FORMAT: str = "<8sHHI"
DBPHASH_HEADER_PREFIX_BYTES: int = 16
DBPHASH_MEDIA_COUNT_BYTE_OFFSET: int = 12
DBPHASH_MEDIA_COUNT_BYTE_LEN: int = 4
DBPHASH_MAGIC_PREFIX: bytes = b"DBSRT"

AGENT_HTTP_USER_AGENT: str = "dbsrt-agent/0.1.0"
DEFAULT_O2SWITCH_API_KEY_PASS_ENTRY: str = "services/pydbsrt/o2switch_api_key"
DEFAULT_FREEBOX_PASS_ENTRY: str = "services/freebox/ftp_pass"

# --- Worktree Seedable Dataset Paths (Relative to Repository Root) ---
WORKTREE_SEED_TARGETS: tuple[str, ...] = (
    "data/catalog.dbphash",
    "data/snippets",
    "data/snippets_ahash",
    "data/__PHASH__",
    "data/__AHASH__",
    "data/reports",
    "data/SRT/couplings.json",
)

# --- Snippet Suffixes Stripped When Matching Parent Reference Couplings ---
SNIPPET_STEM_SUFFIXES: tuple[str, ...] = (
    "_mid15s",
    "_start15s",
    "_end15s",
    "_mid30s",
    "_mid60s",
    "_cut15s",
)
CONTAINER_FPS_SUFFIXES: tuple[str, ...] = (
    ".mkv.23.976fps",
    ".mkv.24.0fps",
    ".mkv.25.0fps",
    ".mp4.23.976fps",
    ".mp4.24.0fps",
    ".mp4.25.0fps",
    ".avi.25.0fps",
)
