---
name: pydbsrt-data-access
description: >-
  Resolves, searches, and inspects pydbsrt media containers (Synology NAS video/tvshow, all 3
  Freebox Server disks, local MP4 snippets) and internal datasets (.phash, .ahash by human title
  or hex moviehash, catalog.dbphash, couplings.json, PostgreSQL, batch reports) across Home Wi-Fi,
  4G/5G hotspot, Work VPN, RPi4, and Cloud o2switch. Use immediately whenever a task locates,
  searches, inspects ffprobe streams/codecs/tracks, counts, checks freshness, syncs, or fetches
  project media, hashes, catalogs, couplings, or worktree datasets. Do NOT use for pure Rust
  algorithm math, Lua UI layout, or MkDocs CSS styling unrelated to locating or inspecting data.
---

# pydbsrt Data & Media Access (SSOT Resolver)

Always use `dbsrt_data.py` **in ONE batched `query` call** (plus ONE `inspect` call only when `ffprobe` streams/codecs/tracks are requested) instead of manual `ls`/`find`/`ssh`/`ffprobe` commands or reading scattered project scripts.

## Single-Shot Execution Reflex

1. **Multi-target batched search & freshness (ALWAYS run `python3` directly in ONE call — avoid `task` wrapper truncation)**:
   ```bash
   python3 .agents/skills/pydbsrt-data-access/scripts/dbsrt_data.py query <spec1> [spec2 ...]
   # Generic example (searches containers, sidecar human titles, snippets & returns cross-node freshness):
   python3 .agents/skills/pydbsrt-data-access/scripts/dbsrt_data.py query "freebox:Matrix" "tvshow:Avatar S01E01" "ahash:Matrix"
   ```
   - **Live container & sidecar search** works across ALL networks (`home_lan`, `nomad_wan` 4G/5G, `work_vpn`):
     - `freebox:<query>`, `video:<query>`, `tvshow:<series SxxExx>`: finds exact `.mkv`/`.mp4`/`.avi` paths (auto-descending into series/season/movie folders), byte sizes, safe URIs, and cross-referenced `phash_file`, `ahash_file`, and `coupled_srt`.
     - `ahash:<title_or_16hex>`, `phash:<title_or_16hex>`: accent-insensitive search over `.json` sidecars (`media_name`, `moviehash`, `duration_sec`, `frames`) + filenames, with automatic `o2switch` HTTPS fallback if missing locally.
     - `snippets:<query>`, `couplings:<query>`: matches local MP4 cuts and parent `couplings.json` subtitles.
   - Top-level `workspace`, `network`, and `freshness` (`local`, `rpi4`, `o2switch` counts, byte sizes, `mtime_iso`, `delta_bytes`, `delta_medias`) are ALWAYS included automatically. Never launch multiple `dbsrt_data.py` processes in parallel.

2. **Container `ffprobe` stream inspection (works on Home LAN, 4G/5G Hotspot, and Work VPN)**:
   ```bash
   python3 .agents/skills/pydbsrt-data-access/scripts/dbsrt_data.py inspect <freebox|video|tvshow|snippets> "<path_or_query>"
   ```
   - Accepts either an exact relative `path` or a search title; runs local/LAN `ffprobe` when safe, or delegates transparently to `o2switch` HTTPS `:443` (`/api/file/details`) when on 4G/5G or Corporate VPN. Returns `video` (`codec`, `width`, `height`, `fps`), `duration_sec`, `audio_streams`, `subtitle_languages` / `subtitle_streams`, and `artifacts`.

3. **Other single-purpose verbs** (`status` | `resolve` | `fetch` | `seed-worktree`):
   - Global overview only: `python3 .agents/skills/pydbsrt-data-access/scripts/dbsrt_data.py status`
   - Fetch/sync corpus or unit file: `python3 .agents/skills/pydbsrt-data-access/scripts/dbsrt_data.py fetch <catalog|ahash|phash|couplings|reports> [item] [--dry-run]`
   - Seed git worktree symlinks: `python3 .agents/skills/pydbsrt-data-access/scripts/dbsrt_data.py seed-worktree`

## Strict Execution & Terse Reporting Discipline (Zero Wasted Turns)

1. **Trust the JSON — zero redundant double-checks**:
   Do **NOT** run follow-up `ls`, `stat`, `find`, or manual `ffprobe` commands, do **NOT** re-calculate deltas already in JSON, and do **NOT** open [data-topology-and-traps.md](./references/data-topology-and-traps.md) unless a channel probe errors out or you are extending [sources_registry.json](../../../scripts/data_access/sources_registry.json).
2. **Direct, non-repetitive bullet reporting**:
   Copy exact values (`path`, `size_bytes`, `ahash_file`, `phash_file`, `moviehash`, `duration_sec`, `delta_bytes`, `delta_medias`) into a clean bullet list without prose/table duplication.
3. **Safety invariants**:
   - **NEVER** `stat`/`ls`/`find` `$MEDIA_MOUNT_ROOT/$USER/{Synology,Freebox}` when `cifs_safe_to_stat` is `false` (prevents kernel SMB `D-state` hang on 4G/5G or Corporate `/16` `ppp0` VPN).
   - **NEVER** assume `ssh o2switch` (`:22`) works on 4G/5G CGNAT; `dbsrt_data.py` uses `https_o2switch` (`:443` + `X-API-Key`) automatically for searches, `inspect`, freshness, and `fetch`.
   - **Freebox has 3 disks** (`Disque_1`, `Disque_2`, `Disque_1_1`); direct WAN `:80` is IP-restricted to `o2switch`. Wrapper: [dbsrt_data.py](./scripts/dbsrt_data.py).
