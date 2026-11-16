#!/usr/bin/env python3
"""Black-box CLI entry point for pydbsrt SSOT data and media access."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

# Ensure repository root is importable regardless of current working directory
_REPO_ROOT = Path(__file__).resolve().parents[2]
if str(_REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(_REPO_ROOT))

from scripts.data_access.constants import (  # noqa: E402
    EXIT_OK,
    EXIT_UNAVAILABLE,
    EXIT_USAGE_ERROR,
    VALID_SIMULATED_PROFILES,
)
from scripts.data_access.prober import collect_data_access_status  # noqa: E402
from scripts.data_access.resolver import (  # noqa: E402
    build_or_run_fetch,
    inspect_family_container,
    load_raw_registry,
    resolve_batch_specs,
    resolve_data_family,
)
from scripts.data_access.topology import (  # noqa: E402
    inspect_network_topology,
    resolve_workspace_context,
    seed_worktree_datasets,
)


def _build_common_parser(suppress_defaults: bool) -> argparse.ArgumentParser:
    """Builds shared global flags; subparsers use SUPPRESS so pre-command flags are preserved."""
    common = argparse.ArgumentParser(add_help=False)
    bool_default: Any = argparse.SUPPRESS if suppress_defaults else False
    str_default: Any = argparse.SUPPRESS if suppress_defaults else ""
    prof_default: Any = argparse.SUPPRESS if suppress_defaults else None
    common.add_argument(
        "--force-refresh",
        action="store_true",
        default=bool_default,
        help="Bypass cached topology/probe state.",
    )
    common.add_argument(
        "--simulate-profile",
        choices=VALID_SIMULATED_PROFILES,
        default=prof_default,
        help="Simulate a specific network profile hermetically (zero network/CIFS calls).",
    )
    common.add_argument(
        "--compact",
        action="store_true",
        default=bool_default,
        help="Emit single-line compact JSON.",
    )
    common.add_argument(
        "--human",
        action="store_true",
        default=bool_default,
        help="Render Rich human-readable tables instead of raw JSON.",
    )
    common.add_argument(
        "--repo",
        default=str_default,
        help="Optional path inside target repo or worktree.",
    )
    return common


def _build_parser() -> argparse.ArgumentParser:
    """Constructs the CLI parser accepting global flags in any position."""
    root_common = _build_common_parser(suppress_defaults=False)
    sub_common = _build_common_parser(suppress_defaults=True)

    parser = argparse.ArgumentParser(
        prog="dbsrt_data.py",
        description="Deterministic SSOT Data & Media Access Resolver for pydbsrt.",
        parents=[root_common],
    )
    parser.add_argument(
        "--check",
        action="store_true",
        help="Run fast offline self-check (<50ms) verifying registry schema and kernel topology.",
    )

    sub = parser.add_subparsers(dest="command")
    sub.add_parser(
        "status", parents=[sub_common], help="Inspect network situation, channels, and cross-node freshness."
    )

    p_res = sub.add_parser("resolve", parents=[sub_common], help="Resolve optimal access channel and query items.")
    p_res.add_argument(
        "family", help="Family ID or alias (all, ahash, phash, catalog, freebox, video, tvshow, snippets...)."
    )
    p_res.add_argument("query", nargs="*", default=[], help="Optional filename/title/moviehash query.")

    p_batch = sub.add_parser(
        "query",
        parents=[sub_common],
        help="Resolve multiple 'family[:search]' targets + global freshness in ONE deduplicated call.",
    )
    p_batch.add_argument(
        "specs",
        nargs="+",
        help="One or more specs, e.g.: freebox:Matrix tvshow:'Andor S01E01' ahash:Hopital",
    )

    p_insp = sub.add_parser(
        "inspect",
        parents=[sub_common],
        help="Inspect ffprobe streams (codec, resolution, duration, tracks) of a local/NAS/Freebox container.",
    )
    p_insp.add_argument("family", help="Media source family (freebox, video, tvshow, snippets).")
    p_insp.add_argument("target", nargs="+", help="Relative container path or search query.")

    p_fetch = sub.add_parser("fetch", parents=[sub_common], help="Fetch dataset or unit file from authoritative node.")
    p_fetch.add_argument("family", help="Target family (catalog, ahash, phash, couplings, reports).")
    p_fetch.add_argument("item", nargs="?", default="", help="Optional unit filename inside corpus.")
    p_fetch.add_argument("--dest", default="", help="Optional destination path override.")
    p_fetch.add_argument("--dry-run", action="store_true", help="Print transfer command without executing.")

    p_seed = sub.add_parser(
        "seed-worktree", parents=[sub_common], help="Symlink unversioned datasets from parent repo."
    )
    p_seed.add_argument("--dry-run", action="store_true", help="Preview symlinks without creating them.")
    return parser


def _emit_json(payload: dict[str, Any], compact: bool) -> None:
    """Writes structured JSON exclusively to stdout."""
    if compact:
        print(json.dumps(payload, ensure_ascii=False))
    else:
        print(json.dumps(payload, indent=2, ensure_ascii=False))


def _emit_compact_lines_json(payload: dict[str, Any]) -> None:
    """Emits one compact JSON line per top-level key (<8KB total, <4KB/line, zero truncation)."""
    items = [f'"{k}":{json.dumps(v, ensure_ascii=False, separators=(",", ":"))}' for k, v in payload.items()]
    print("{\n" + ",\n".join(items) + "\n}")


def _run_self_check(start_dir: Path | None, compact: bool) -> int:
    """Executes offline schema and kernel topology validation with zero network packets."""
    raw_reg = load_raw_registry()
    ws = resolve_workspace_context(start_dir)
    net = inspect_network_topology()
    families = [f["id"] for f in raw_reg.get("families", [])]
    disks = [d["id"] for d in raw_reg.get("freebox_disks", [])]
    ok = bool(families) and len(disks) >= 3
    _emit_json(
        {
            "check": "ok" if ok else "failed",
            "families_count": len(families),
            "freebox_disks_count": len(disks),
            "detected_profile": net["profile"],
            "cifs_safe_to_stat": net["cifs_safe_to_stat"],
            "is_worktree": ws.is_worktree,
            "effective_data_dir": str(ws.effective_data_dir),
        },
        compact,
    )
    return EXIT_OK if ok else EXIT_UNAVAILABLE


def _handle_resolve_cmd(args: argparse.Namespace, ws: Any, status_payload: dict[str, Any]) -> int:
    """Executes single-family resolve subcommand."""
    q_str = " ".join(args.query) if isinstance(args.query, list) else str(args.query)
    resolved = resolve_data_family(ws, status_payload, args.family, query=q_str)
    if getattr(args, "human", False):
        from scripts.data_access.human_view import render_human_generic

        render_human_generic(f"🔎 Resolve: {args.family}", resolved)
    else:
        _emit_json(resolved, args.compact)
    return EXIT_USAGE_ERROR if "error" in resolved else EXIT_OK


def _handle_inspect_cmd(args: argparse.Namespace, ws: Any, status_payload: dict[str, Any]) -> int:
    """Executes container ffprobe inspect subcommand."""
    tgt_str = " ".join(args.target) if isinstance(args.target, list) else str(args.target)
    inspected = inspect_family_container(ws, status_payload, args.family, tgt_str)
    if getattr(args, "human", False):
        from scripts.data_access.human_view import render_human_inspect

        render_human_inspect(inspected)
    elif args.compact:
        _emit_json(inspected, True)
    else:
        _emit_compact_lines_json(inspected)
    return EXIT_OK if inspected.get("ok") else EXIT_UNAVAILABLE


def _handle_status_cmd(args: argparse.Namespace, status_payload: dict[str, Any]) -> int:
    """Emits status payload as Rich human dashboard or JSON."""
    if getattr(args, "human", False):
        from scripts.data_access.human_view import render_human_status

        render_human_status(status_payload)
    else:
        _emit_json(status_payload, args.compact)
    return EXIT_OK


def _handle_query_cmd(args: argparse.Namespace, ws: Any, status_payload: dict[str, Any]) -> int:
    """Executes batched multi-family query subcommand."""
    batched = resolve_batch_specs(ws, status_payload, args.specs)
    if getattr(args, "human", False):
        from scripts.data_access.human_view import render_human_query

        render_human_query(batched)
    else:
        _emit_compact_lines_json(batched)
    return EXIT_USAGE_ERROR if any("error" in v for v in batched["results"].values()) else EXIT_OK


def _handle_fetch_cmd(args: argparse.Namespace, ws: Any, status_payload: dict[str, Any]) -> int:
    """Executes dataset/artifact fetch subcommand."""
    fetched = build_or_run_fetch(
        ws,
        status_payload,
        args.family,
        item=args.item,
        dest_override=args.dest,
        dry_run=args.dry_run,
    )
    if getattr(args, "human", False):
        from scripts.data_access.human_view import render_human_generic

        render_human_generic(f"⬇️ Fetch: {args.family}", fetched)
    else:
        _emit_json(fetched, args.compact)
    return EXIT_OK if fetched.get("ok") else EXIT_UNAVAILABLE


def _dispatch_probed_command(
    args: argparse.Namespace,
    ws: Any,
    status_payload: dict[str, Any],
) -> int:
    """Dispatches status, resolve, query, inspect, or fetch subcommands."""
    if args.command == "status":
        return _handle_status_cmd(args, status_payload)
    if args.command == "resolve":
        return _handle_resolve_cmd(args, ws, status_payload)
    if args.command == "query":
        return _handle_query_cmd(args, ws, status_payload)
    if args.command == "inspect":
        return _handle_inspect_cmd(args, ws, status_payload)
    if args.command == "fetch":
        return _handle_fetch_cmd(args, ws, status_payload)
    return EXIT_USAGE_ERROR


def main(argv: list[str] | None = None) -> int:
    """CLI dispatch routine."""
    parser = _build_parser()
    args = parser.parse_args(argv)
    start_dir = Path(args.repo).resolve() if args.repo else Path.cwd()

    if args.check:
        return _run_self_check(start_dir, args.compact)

    if not args.command:
        parser.print_help(sys.stderr)
        return EXIT_USAGE_ERROR

    ws = resolve_workspace_context(start_dir)
    if args.command == "seed-worktree":
        _emit_json(seed_worktree_datasets(ws, dry_run=args.dry_run), args.compact)
        return EXIT_OK

    status_payload = collect_data_access_status(
        ws,
        force_refresh=args.force_refresh,
        simulated_profile=args.simulate_profile,
    )
    return _dispatch_probed_command(args, ws, status_payload)


if __name__ == "__main__":
    sys.exit(main())
