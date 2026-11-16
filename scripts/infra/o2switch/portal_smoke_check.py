#!/usr/bin/env python3
"""Zero-Pip smoke verification script for staged pydbsrt Portal on o2switch."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path
from typing import Any

MIN_SOURCES_HTML_BYTES: int = 10_000
MIN_SOURCES_COUNT: int = 4
SMOKE_TIMEOUT_SECS: float = 20.0
SMOKE_LIMIT: int = 3


def _run_json_cmd(cmd: list[str], cwd: Path, env: dict[str, str]) -> dict[str, Any]:
    """Executes a portal CLI command and parses its JSON stdout payload."""
    proc = subprocess.run(
        cmd,
        cwd=str(cwd),
        env=env,
        capture_output=True,
        text=True,
        timeout=SMOKE_TIMEOUT_SECS,
        check=False,
    )
    if proc.returncode != 0:
        raise RuntimeError(f"Command failed ({proc.returncode}): {' '.join(cmd)}\nSTDERR: {proc.stderr.strip()}")
    payload = json.loads(proc.stdout)
    if not isinstance(payload, dict):
        raise RuntimeError(f"Expected JSON object from {' '.join(cmd)}")
    return payload


def _verify_static_asset(preview_root: Path) -> None:
    """Verifies presence and size of the staged sources.html SPA."""
    sources_html = preview_root / "src" / "pydbsrt" / "portal" / "static" / "sources.html"
    if not sources_html.is_file():
        raise RuntimeError(f"Missing static asset: {sources_html}")
    size_bytes = sources_html.stat().st_size
    if size_bytes < MIN_SOURCES_HTML_BYTES:
        raise RuntimeError(f"Unexpectedly small sources.html ({size_bytes} bytes)")
    print(f"✓ Static SPA OK: sources.html ({size_bytes} bytes)")


def _verify_cli_and_module_metadata(py_bin: str, shim: Path, preview_root: Path, env: dict[str, str]) -> None:
    """Verifies both shim CLI and `python -m pydbsrt.portal` entrypoints."""
    meta_shim = _run_json_cmd([py_bin, str(shim), "--metadata"], preview_root, env)
    if len(meta_shim.get("sources", [])) < MIN_SOURCES_COUNT:
        raise RuntimeError(f"Insufficient sources in shim --metadata: {meta_shim}")
    meta_mod = _run_json_cmd([py_bin, "-m", "pydbsrt.portal", "--metadata"], preview_root, env)
    if meta_mod.get("detected_env") != meta_shim.get("detected_env"):
        raise RuntimeError("Mismatch between shim and -m pydbsrt.portal detected_env")
    print(f"✓ Entrypoints OK: detected_env={meta_shim.get('detected_env')}, sources={len(meta_shim['sources'])}")


def _verify_data_endpoints(py_bin: str, shim: Path, preview_root: Path, env: dict[str, str]) -> None:
    """Verifies PostgreSQL CLI fallback, tripartite pHash catalog, trajectories, and WAN FTP probe."""
    assets = _run_json_cmd([py_bin, str(shim), "--assets", "--limit", str(SMOKE_LIMIT)], preview_root, env)
    if assets.get("status") != "ok" or not assets.get("connected"):
        raise RuntimeError(f"Assets PG CLI check failed: {assets}")
    print(f"✓ PostgreSQL CLI fallback OK: {assets.get('pg_version')} (total={assets.get('summary', {}).get('total')})")

    phashes = _run_json_cmd([py_bin, str(shim), "--phashes", "--limit", str(SMOKE_LIMIT)], preview_root, env)
    summary = phashes.get("summary", {})
    if phashes.get("status") != "ok" or summary.get("catalog_count", 0) <= 0:
        raise RuntimeError(f"pHash tripartite check failed: {phashes}")
    print(
        "✓ pHash tripartite OK: "
        f"raw={summary.get('raw_count')}, catalog={summary.get('catalog_count')}, aligned={summary.get('aligned_3_3')}"
    )

    trajectories = _run_json_cmd([py_bin, str(shim), "--trajectories", "--limit", str(SMOKE_LIMIT)], preview_root, env)
    if trajectories.get("status") != "ok":
        raise RuntimeError(f"Trajectories check failed: {trajectories}")
    print(f"✓ Trajectories OK: total_items={trajectories.get('total_items')}")

    probe = _run_json_cmd([py_bin, str(shim), "--probe", "synology-video", "wan_ftp"], preview_root, env)
    if "status" not in probe:
        raise RuntimeError(f"Invalid WAN FTP probe response: {probe}")
    print(f"✓ Probe synology-video/wan_ftp: status={probe.get('status')}, files={probe.get('files_count')}")


def main() -> int:
    """Runs isolated Zero-Pip portal smoke suite inside staged preview directory."""
    preview_root = Path(__file__).resolve().parents[3]
    shim = preview_root / "scripts" / "portal_server.py"
    py_bin = sys.executable

    env = os.environ.copy()
    env["PYTHONPATH"] = f"{preview_root / 'src'}:{preview_root}"

    _verify_static_asset(preview_root)
    _verify_cli_and_module_metadata(py_bin, shim, preview_root, env)
    _verify_data_endpoints(py_bin, shim, preview_root, env)
    print("✓ o2switch isolated portal canary PASSED 100% (Zero-Pip, zero prod impact).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
