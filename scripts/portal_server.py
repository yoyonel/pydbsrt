#!/usr/bin/env python3
"""Backward-compatible CLI entrypoint and re-export shim for pydbsrt Portal.

Canonical implementation resides in `src/pydbsrt/portal/`.
"""

from __future__ import annotations

import sys
from pathlib import Path

REPO_ROOT: Path = Path(__file__).resolve().parents[1]
SRC_ROOT: Path = REPO_ROOT / "src"
for _p in (str(REPO_ROOT), str(SRC_ROOT)):
    if _p not in sys.path:
        sys.path.insert(0, _p)

from pydbsrt.portal import (  # noqa: E402, F401
    BYTES_PER_GB,
    BYTES_PER_KB,
    BYTES_PER_MB,
    BYTES_PER_TB,
    DEFAULT_GATEWAY_LAN_URL,
    DEFAULT_GATEWAY_WAN_URL,
    DEFAULT_PORT,
    PhashesQueryCriteria,
    PhashReconciliationContext,
    PortalRequestHandler,
    ReusableThreadingTCPServer,
    TrajectoriesQueryCriteria,
    _build_sidecar_name_index,
    _compute_trajectory_kpi,
    _is_cifs_safe_to_stat,
    _is_excluded_portal_entry,
    _is_valid_trajectory_file,
    _parse_mlsd_item,
    _parse_nlst_item,
    _read_disk_cache,
    _resolve_freebox_endpoint,
    _resolve_phash_moviehash,
    _write_disk_cache,
    execute_single_probe,
    format_human_size,
    format_seconds_duration,
    get_pg_asset_detail,
    get_pg_assets,
    get_sources_metadata,
    get_trajectory_record_detail,
    inspect_file_details,
    inspect_phash_microscopic,
    list_source_files,
    list_trajectory_audit_files,
    main,
    parse_media_filename,
    probe_freebox_ftp_profile,
    probe_ftp_profile,
    probe_http_gateway_profile,
    probe_local_fs_profile,
    query_phashes_list,
    query_trajectory_records,
    run_on_demand_trajectory,
    run_server,
    strip_accents,
)

if __name__ == "__main__":
    main()
