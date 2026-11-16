"""Human-readable Rich terminal renderers for pydbsrt SSOT data access outputs."""

from __future__ import annotations

from typing import Any

from rich import box
from rich.console import Console
from rich.panel import Panel
from rich.table import Table
from rich.text import Text

from scripts.data_access.constants import (
    BYTES_PER_GB,
    BYTES_PER_KB,
    BYTES_PER_MB,
    SECONDS_PER_DAY,
    SECONDS_PER_HOUR,
    SECONDS_PER_MINUTE,
)

DELTA_PRECISION_HOURS: int = 1
MAX_SUBTITLE_LANG_PREVIEW: int = 8
MAX_HUMAN_PATH_DISPLAY_CHARS: int = 72


def _fmt_bytes(num_bytes: int) -> str:
    """Formats byte count into human-readable GiB/MiB/KiB/B string."""
    if num_bytes <= 0:
        return "0 B"
    if num_bytes >= BYTES_PER_GB:
        return f"{num_bytes / BYTES_PER_GB:.2f} GiB"
    if num_bytes >= BYTES_PER_MB:
        return f"{num_bytes / BYTES_PER_MB:.1f} MiB"
    if num_bytes >= BYTES_PER_KB:
        return f"{num_bytes / BYTES_PER_KB:.1f} KiB"
    return f"{num_bytes} B"


def _fmt_duration(seconds: float) -> str:
    """Formats duration in seconds into HHhMMmSSs or MMmSSs."""
    total = int(round(max(0.0, seconds)))
    hours, rem = divmod(total, SECONDS_PER_HOUR)
    mins, secs = divmod(rem, SECONDS_PER_MINUTE)
    if hours > 0:
        return f"{hours}h{mins:02d}m{secs:02d}s"
    if mins > 0:
        return f"{mins}m{secs:02d}s"
    return f"{secs}s"


def _fmt_age(seconds: int | float) -> str:
    """Formats delta seconds into days/hours/minutes."""
    secs = int(max(0.0, float(seconds)))
    if secs >= SECONDS_PER_DAY:
        return f"{secs // SECONDS_PER_DAY}j {(secs % SECONDS_PER_DAY) // SECONDS_PER_HOUR}h"
    if secs >= SECONDS_PER_HOUR:
        return f"{secs / SECONDS_PER_HOUR:.{DELTA_PRECISION_HOURS}f}h"
    if secs >= SECONDS_PER_MINUTE:
        return f"{secs // SECONDS_PER_MINUTE}m"
    return f"{secs}s"


def _truncate_middle(text: str, limit: int = MAX_HUMAN_PATH_DISPLAY_CHARS) -> str:
    """Truncates long paths in the middle while preserving filename tail."""
    if len(text) <= limit:
        return text
    half = (limit - 3) // 2
    return f"{text[:half]}...{text[-(limit - 3 - half) :]}"


def _build_network_header_panel(payload: dict[str, Any]) -> Panel:
    """Builds the top summary panel for network profile, CIFS safety, workspace, and cache."""
    net = payload.get("network", {})
    ws = payload.get("workspace", {})
    cache = payload.get("cache", {})

    profile = str(net.get("profile") or "unknown")
    host_env = str(net.get("host_env") or "workstation")
    cifs_safe = bool(net.get("cifs_safe_to_stat", False))
    cifs_badge = "[bold green]SAFE (Home LAN)[/]" if cifs_safe else "[bold yellow]BLOQUÉ (Garde-fou VPN/WAN)[/]"
    cache_state = "[cyan]HIT[/]" if cache.get("hit") else "[magenta]MISS[/]"
    cache_age = f"{cache.get('age_seconds', 0)}s / {cache.get('lookup_ms', 0)} ms"
    wt_label = "worktree" if ws.get("is_worktree") else "repo principal"

    body = (
        f"[bold]Profil :[/] [cyan]{profile}[/] ({host_env})   "
        f"[bold]CIFS :[/] {cifs_badge}   "
        f"[bold]Cache :[/] {cache_state} ({cache_age})\n"
        f"[bold]Workspace :[/] {ws.get('active_repo', '-')} ([dim]{wt_label}[/])"
    )
    unseeded = ws.get("unseeded_items") or []
    if unseeded:
        body += f"\n[bold yellow]Non seedés ({len(unseeded)}) :[/] {', '.join(unseeded)}"
    return Panel(body, title="🌐 Situation Réseau & Workspace", border_style="cyan", box=box.ROUNDED)


def _format_channel_status(info: dict[str, Any]) -> tuple[str, str]:
    """Returns (rich_status_badge, detail_str) for one channel probe entry."""
    if info.get("accessible"):
        note = str(info.get("note") or "")
        extra = f"{info['items_count']} items" if "items_count" in info else note
        return "[bold green]OK[/]", extra
    err = str(info.get("error") or "unreachable")
    if err.startswith("skipped_"):
        return "[dim]SKIP[/]", err
    if err == "blocked_by_stale_cifs_hang_guard":
        return "[yellow]GUARD[/]", err
    return "[bold red]OFF[/]", err


def _build_channels_table(channels: dict[str, Any]) -> Table:
    """Builds the Rich table of probed network/storage channels."""
    table = Table(title="📡 Canaux d'Accès", box=box.SIMPLE_HEAVY, header_style="bold yellow")
    table.add_column("Canal", style="bold")
    table.add_column("État", justify="center", width=8)
    table.add_column("Latence", justify="right", width=10)
    table.add_column("Détail / Raison")

    for key, raw in channels.items():
        if not isinstance(raw, dict):
            continue
        badge, detail = _format_channel_status(raw)
        label = str(raw.get("label") or key)
        lat = f"{raw.get('latency_ms', 0.0):.1f} ms"
        table.add_row(label, badge, lat, detail)
    return table


def _node_catalog_cell(node_data: dict[str, Any] | None, field: str) -> str:
    """Formats one catalog or corpus cell for a single node."""
    if not isinstance(node_data, dict):
        return "[dim]-[/]"
    entry = node_data.get(field)
    if not isinstance(entry, dict):
        return "[dim]-[/]"
    if "media_count" in entry:
        if not entry.get("exists", True):
            return "[dim]absent[/]"
        return f"{entry.get('media_count', 0)} médias ({_fmt_bytes(int(entry.get('size_bytes') or 0))})"
    return f"{entry.get('count', 0)} fichiers"


def _status_badge(status: str) -> str:
    """Colorizes a freshness/backup status string."""
    if status in ("UP_TO_DATE", "BACKED_UP"):
        return f"[bold green]{status}[/]"
    if status in ("STALE_LOCAL", "PARTIAL_LOCAL"):
        return f"[bold yellow]{status}[/]"
    return f"[bold red]{status}[/]"


def _build_freshness_table(freshness: dict[str, Any]) -> Table:
    """Builds the Rich cross-node corpus freshness comparison table."""
    nodes = freshness.get("nodes", {})
    table = Table(title="📦 Fraîcheur Corpus & Catalogues", box=box.SIMPLE_HEAVY, header_style="bold yellow")
    table.add_column("Dataset", style="bold")
    table.add_column("Local")
    table.add_column("RPi4")
    table.add_column("o2switch")
    table.add_column("Statut / Delta")

    cat = freshness.get("packed_catalog", {})
    cat_delta = (
        f"+{cat.get('delta_medias', 0)} médias / {_fmt_age(cat.get('delta_seconds', 0))}"
        if cat.get("status") != "UP_TO_DATE"
        else "synchro"
    )
    table.add_row(
        "catalog.dbphash",
        _node_catalog_cell(nodes.get("local"), "catalog_dbphash"),
        _node_catalog_cell(nodes.get("rpi4"), "catalog_dbphash"),
        _node_catalog_cell(nodes.get("o2switch"), "catalog_dbphash"),
        f"{_status_badge(str(cat.get('status', '?')))} ({cat_delta})",
    )

    acat = freshness.get("packed_ahash_catalog", {})
    table.add_row(
        "catalog.dbahash",
        _node_catalog_cell(nodes.get("local"), "catalog_dbahash"),
        _node_catalog_cell(nodes.get("rpi4"), "catalog_dbahash"),
        _node_catalog_cell(nodes.get("o2switch"), "catalog_dbahash"),
        _status_badge(str(acat.get("status", "?"))),
    )

    for label, corp_key, node_field in (
        ("Corpus .phash", "phash_corpus", "phash_files"),
        ("Corpus .ahash", "ahash_corpus", "ahash_files"),
    ):
        corp = freshness.get(corp_key, {})
        miss = corp.get("missing_locally", 0)
        delta_txt = f"manquants local: {miss}" if miss else "complet"
        table.add_row(
            label,
            _node_catalog_cell(nodes.get("local"), node_field),
            _node_catalog_cell(nodes.get("rpi4"), node_field),
            _node_catalog_cell(nodes.get("o2switch"), node_field),
            f"{_status_badge(str(corp.get('status', '?')))} ({delta_txt})",
        )
    return table


def _build_backup_panel(backup: dict[str, Any]) -> Panel:
    """Builds concise backup & snapshot health panel."""
    status = str(backup.get("status") or "UNKNOWN")
    snap = str(backup.get("latest_snapshot_id") or "-")
    reps = ", ".join(backup.get("verified_replicas") or []) or "-"
    evicted = backup.get("evicted_datasets") or []
    ev_txt = ", ".join(str(e) for e in evicted) if evicted else "aucun"
    body = (
        f"[bold]Statut :[/] {_status_badge(status)}   "
        f"[bold]Snapshot :[/] [cyan]{snap}[/]   "
        f"[bold]Réplicas vérifiés :[/] {reps}   "
        f"[bold]Évictions :[/] {ev_txt}"
    )
    return Panel(body, title="🛡️ Sauvegardes Multi-Tiers", border_style="green", box=box.ROUNDED)


def render_human_status(payload: dict[str, Any], console: Console | None = None) -> None:
    """Renders `dbsrt_data.py status --human` as structured Rich panels and tables."""
    out = console or Console()
    out.print(_build_network_header_panel(payload))
    out.print(_build_channels_table(payload.get("channels", {})))
    out.print(_build_freshness_table(payload.get("freshness", {})))
    if "backup_state" in payload:
        out.print(_build_backup_panel(payload["backup_state"]))


def _format_query_match_summary(spec_entry: dict[str, Any]) -> tuple[str, str, str]:
    """Extracts (via_channel, target_summary, meta_summary) from one batch query result."""
    qr = spec_entry.get("query_result") or {}
    via = str(qr.get("via_channel") or (spec_entry.get("recommended_channel") or {}).get("id") or "-")
    matches = qr.get("matches") or []
    if not matches:
        return via, "[dim]0 résultat[/]", "-"

    top = matches[0]
    name = str(top.get("media_name") or top.get("path") or top.get("filename") or "-")
    extra = f" [dim](+{len(matches) - 1})[/]" if len(matches) > 1 else ""
    size_val = int(top.get("size_bytes") or 0)
    size_txt = _fmt_bytes(size_val) if size_val > 0 else "-"
    dur = top.get("duration_sec")
    if dur:
        size_txt = f"{size_txt} • {_fmt_duration(float(dur))}"

    tags: list[str] = []
    if top.get("moviehash"):
        tags.append(f"mh:{top['moviehash']}")
    if top.get("phash_file"):
        tags.append("pHash")
    if top.get("ahash_file"):
        tags.append("aHash")
    if top.get("coupled_srt") or top.get("parent_media_coupled"):
        cnt = top.get("coupled_subtitles_count") or (top.get("parent_coupling") or {}).get("certified_subtitles_count")
        tags.append(f"[green]SRT({cnt or 'ok'})[/]")
    meta = f"{size_txt} | {' '.join(tags) if tags else '-'}"
    return via, f"{_truncate_middle(name)}{extra}", meta


def render_human_query(payload: dict[str, Any], console: Console | None = None) -> None:
    """Renders `dbsrt_data.py query --human` as concise Rich summary + results table."""
    out = console or Console()
    out.print(_build_network_header_panel(payload))
    out.print(_build_freshness_table(payload.get("freshness", {})))

    table = Table(title="🔍 Résultats Multi-Sources", box=box.SIMPLE_HEAVY, header_style="bold yellow")
    table.add_column("Requête", style="bold cyan")
    table.add_column("Canal", style="magenta")
    table.add_column("Cible principale")
    table.add_column("Taille / Empreintes & Couplage")

    for spec, entry in (payload.get("results") or {}).items():
        if not isinstance(entry, dict):
            continue
        if "error" in entry:
            table.add_row(spec, "[red]ERR[/]", f"[red]{entry['error']}[/]", "-")
            continue
        via, target, meta = _format_query_match_summary(entry)
        table.add_row(spec, via, target, meta)
    out.print(table)


def _build_inspect_audio_table(audios: list[dict[str, Any]]) -> Table:
    """Builds Rich table of audio tracks for container inspection."""
    table = Table(title="🔊 Pistes Audio", box=box.SIMPLE_HEAVY, header_style="bold yellow")
    table.add_column("#", justify="right", width=4)
    table.add_column("Codec", style="cyan")
    table.add_column("Canaux", justify="right")
    table.add_column("Langue", style="green")
    table.add_column("Titre")
    for st in audios:
        layout = str(st.get("channel_layout") or st.get("channels") or "-")
        table.add_row(
            str(st.get("index", "-")),
            str(st.get("codec") or "-"),
            layout,
            str(st.get("language") or "und"),
            str(st.get("title") or "-"),
        )
    return table


def _build_inspect_subs_table(payload: dict[str, Any]) -> Table:
    """Builds Rich table of subtitle tracks for container inspection."""
    subs = payload.get("subtitle_streams") or []
    total = int(payload.get("subtitle_streams_count") or len(subs))
    langs = str(payload.get("subtitle_languages") or "-")
    title = f"💬 Sous-titres ({total} pistes • langues: {langs})"
    table = Table(title=title, box=box.SIMPLE_HEAVY, header_style="bold yellow")
    table.add_column("#", justify="right", width=4)
    table.add_column("Codec", style="cyan")
    table.add_column("Langue", style="green")
    table.add_column("Forcé", justify="center")
    table.add_column("Titre")
    for st in subs:
        forced = "[yellow]oui[/]" if st.get("forced") else "non"
        table.add_row(
            str(st.get("index", "-")),
            str(st.get("codec") or "-"),
            str(st.get("language") or "und"),
            forced,
            str(st.get("title") or "-"),
        )
    return table


def render_human_inspect(payload: dict[str, Any], console: Console | None = None) -> None:
    """Renders `dbsrt_data.py inspect --human` with video summary and audio/subtitle tables."""
    out = console or Console()
    if not payload.get("ok"):
        err = str(payload.get("error") or "unknown_error")
        target = str(payload.get("path") or payload.get("query") or "-")
        out.print(
            Panel(
                f"[bold red]Erreur :[/] {err}\n[bold]Famille :[/] {payload.get('family', '-')}\n[bold]Cible :[/] {target}",
                title="❌ Inspection ffprobe échouée",
                border_style="red",
                box=box.ROUNDED,
            )
        )
        return

    vid = payload.get("video") or {}
    art = payload.get("artifacts") or {}
    vid_str = (
        f"{vid.get('codec', '?')} {vid.get('width', '?')}x{vid.get('height', '?')} "
        f"@ {vid.get('fps', '?')} fps ({vid.get('pix_fmt', '?')})"
    )
    coupled = (
        f"[bold green]OUI ({art.get('coupled_subtitles_count', 0)} SRT)[/]" if art.get("coupled_srt") else "[dim]non[/]"
    )
    summary = Text.from_markup(
        f"[bold]Famille :[/] [cyan]{payload.get('family')}[/]   "
        f"[bold]Via :[/] [magenta]{payload.get('via')}[/]\n"
        f"[bold]Chemin :[/] {payload.get('path')}\n"
        f"[bold]Taille :[/] {_fmt_bytes(int(payload.get('size_bytes') or 0))}   "
        f"[bold]Durée :[/] {_fmt_duration(float(payload.get('duration_sec') or 0.0))}   "
        f"[bold]Vidéo :[/] {vid_str}\n"
        f"[bold]Moviehash :[/] {art.get('moviehash') or '-'}   "
        f"[bold]pHash :[/] {'oui' if art.get('phash_file') else 'non'}   "
        f"[bold]aHash :[/] {'oui' if art.get('ahash_file') else 'non'}   "
        f"[bold]Couplage SRT :[/] {coupled}"
    )
    out.print(Panel(summary, title="🎬 Inspection Conteneur ffprobe", border_style="green", box=box.ROUNDED))
    if payload.get("audio_streams"):
        out.print(_build_inspect_audio_table(payload["audio_streams"]))
    if payload.get("subtitle_streams"):
        out.print(_build_inspect_subs_table(payload))


def render_human_generic(title: str, payload: dict[str, Any], console: Console | None = None) -> None:
    """Renders compact key/value Rich table for resolve/fetch subcommands."""
    out = console or Console()
    table = Table(title=title, box=box.SIMPLE_HEAVY, header_style="bold yellow")
    table.add_column("Champ", style="bold cyan")
    table.add_column("Valeur")
    for key, val in payload.items():
        if isinstance(val, dict | list):
            summary = f"{len(val)} entrées" if isinstance(val, list) else f"{len(val)} clés"
            table.add_row(str(key), summary)
        else:
            table.add_row(str(key), str(val))
    out.print(table)
