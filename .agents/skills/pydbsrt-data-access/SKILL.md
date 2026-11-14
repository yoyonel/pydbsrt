---
name: pydbsrt-data-access
description: >-
  Resolves where pydbsrt media sources (Synology NAS video/tvshow, all 3 Freebox Server disks,
  local MP4 snippets) and internal datasets (.phash, .ahash, catalog.dbphash, couplings.json,
  PostgreSQL, batch reports) live and which exact channel accesses them safely across Home Wi-Fi,
  4G/5G hotspot, Work VPN, RPi4, and Cloud o2switch. Use immediately whenever a task locates,
  counts, checks freshness, syncs, or fetches project media, hashes, catalogs, couplings, or
  worktree datasets. Do NOT use for pure Rust algorithm math, Lua UI layout, or MkDocs CSS styling
  unrelated to locating or transferring data.
---

# pydbsrt Data & Media Access (SSOT Resolver)

Always run the SSOT resolver **in ONE single batched call** instead of spawning multiple parallel CLI calls, manual `ls`/`find`/`ssh` checks, or reading scattered project scripts.

## Single-Shot Execution Reflex

1. **Multi-target batched query (ALWAYS run `python3` directly in ONE call — avoid `task` wrapper overhead/truncation)**:
   ```bash
   python3 .agents/skills/pydbsrt-data-access/scripts/dbsrt_data.py query <spec1> [spec2 ...]
   # Generic example (top-level `freshness` ALREADY includes catalog, phash, ahash, snippets_ahash, couplings across all nodes!):
   python3 .agents/skills/pydbsrt-data-access/scripts/dbsrt_data.py query video snippets:Matrix
   ```
   - Top-level `workspace`, `network`, and `freshness` (`local`, `rpi4`, `o2switch` counts, byte sizes, `mtime_iso`, `delta_bytes`, `delta_medias`) are ALWAYS returned automatically — only pass `specs` for specific source routing (e.g. `freebox`, `video`) or item searches (e.g. `snippets:<term>`), keeping stdout `< 8 KB` inline.
   - Do **NOT** pass `--force-refresh` unless explicitly asked, and NEVER launch multiple `dbsrt_data.py` processes in parallel.

2. **Other single-purpose verbs** (CLI or `task data:status | resolve | query | fetch | seed-worktree`):
   - Global overview only: `python3 .agents/skills/pydbsrt-data-access/scripts/dbsrt_data.py status`
   - Fetch/sync corpus or file: `python3 .agents/skills/pydbsrt-data-access/scripts/dbsrt_data.py fetch <family> [item] --dry-run`
   - Seed git worktree symlinks: `python3 .agents/skills/pydbsrt-data-access/scripts/dbsrt_data.py seed-worktree`
   - Supported family IDs / aliases: `all`, `video` (`synology-video`), `tvshow` (`synology-tvshow`), `freebox` (`freebox-disks`), `snippets` (`local-snippets`), `snippets-ahash`, `phash` (`phash-files`), `ahash` (`ahash-files`), `catalog` (`packed-catalog`), `couplings` (`certified-couplings`), `db` (`postgresql-db`), `reports` (`reports-and-inventories`), `fixtures` (`test-fixtures`).

## Strict Execution & Terse Reporting Discipline (Zero Wasted Turns)

1. **Trust the live JSON — zero redundant double-checks**:
   `dbsrt_data.py` directly inspects local files, worktree symlinks (`active_worktree_direct`), 16-byte DBSRT v2 `catalog.dbphash` headers, `couplings.json`, and remote SSH/HTTPS nodes. Do **NOT** run follow-up `ls`, `stat`, or `find` commands, do **NOT** re-compute byte/time deltas already present in JSON, and do **NOT** open [data-topology-and-traps.md](./references/data-topology-and-traps.md) unless a channel probe errors out or you are adding a source to [sources_registry.json](../../../scripts/data_access/sources_registry.json).
2. **Direct, non-repetitive bullet reporting**:
   - Copy values directly from the JSON (`delta_bytes`, `delta_medias`, `mtime_iso`, `missing_locally`) without manual recalculation.
   - Format the answer as a clean bullet list (one bullet per node, disk, or matched file; state shared subtitle languages once; do not duplicate prose summaries and tables).
3. **Safety invariants**:
   - **NEVER** `stat`/`ls`/`find` `$MEDIA_MOUNT_ROOT/$USER/{Synology,Freebox}` when `cifs_safe_to_stat` is `false` (prevents kernel SMB `D-state` hang on 4G/5G or Corporate `/16` `ppp0` VPN).
   - **NEVER** assume `ssh o2switch` (`:22`) works on 4G/5G CGNAT; rely on `https_o2switch` (`:443` + `X-API-Key`) or `wan_ftp_synology`.
   - **Freebox has 3 disks** (`Disque_1`, `Disque_2`, `Disque_1_1`); legacy `portal_server.py` UI omits `Disque_1_1`, and direct WAN `:80` is IP-restricted to `o2switch`.
   - Wrapper script: [dbsrt_data.py](./scripts/dbsrt_data.py).
