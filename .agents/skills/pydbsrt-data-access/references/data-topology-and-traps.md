# Multi-Network Data Topology, Empirical Traps & Extension Guide

## 1. Empirically Verified Channel Matrix Across the 4 Workstation Contexts

| Channel / Endpoint | Home Wi-Fi (`home_lan`) | 4G/5G Hotspot (`nomad_wan`) | Work Wi-Fi + VPN (`work_vpn`) | Cloud `o2switch` (`o2switch`) | Appliance `rpi4` (`rpi4`) |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **Local NVMe (`data/` or parent worktree)** | Accessible (`<1 ms`) | Accessible (`<1 ms`) | Accessible (`<1 ms`) | `~/dbsrt/data/` | `~/dbsrt/data/` |
| **Local CIFS (`$MEDIA_MOUNT_ROOT/$USER/{Synology,Freebox}`)** | Accessible (`~50 ms`) | **KERNEL HANG TRAP** (stale `/proc/mounts`) | **KERNEL HANG TRAP** (`ppp0` `/16` route hijack) | N/A | `/mnt/freebox/*` (`ro`) |
| **LAN FTP Synology (`<NAS_LAN_IP>:21`)** | Accessible (`~160 ms`) | Unreachable | Blackholed into `ppp0` | N/A | Accessible |
| **WAN FTP Synology (`<NAS_HOST>:21` PASV)** | Accessible (`~335 ms`) | **Accessible (`~420-690 ms`)** | Test live (`wan_ftp_synology`) | Accessible (`~270 ms`) | Accessible |
| **LAN HTTP Freebox Gateway (`<RPI4_LAN_IP>:18089`)** | Accessible (`~43 ms`, JSON) | Unreachable | Blackholed into `ppp0` | N/A | Loopback `:18089` |
| **Direct WAN HTTP Freebox (`<NAS_HOST>:80`)** | Returns HTML (`false`!) | Returns HTML / Filtered (`false`!) | Filtered (`false`!) | **Accessible (`~50 ms`, source-IP NAT)** | N/A |
| **SSH `rpi4` (`<RPI4_LAN_IP>:22`)** | Accessible (`~480 ms`) | Unreachable | Blackholed into `ppp0` | N/A | Localhost |
| **SSH `o2switch` (`sapin.o2switch.net:22`)** | Accessible (`~510 ms`, whitelisted IP) | **TIMEOUT (`:22` dropped by cPanel IP whitelist)** | Accessible if corporate egress IP whitelisted | Localhost | Accessible if whitelisted |
| **HTTPS `o2switch` (`O2SWITCH_URL` `:443` + `X-API-Key`)** | Accessible (`~115 ms`) | **Accessible (`~135-250 ms`, bypasses `:22` block)** | **Accessible (`~135 ms`)** | Unix socket | Accessible |

## 2. Five Infrastructure Traps Never to Repeat

1. **Corporate VPN `ppp0` Subnet Hijack**:
   Corporate PPP VPN injects a `/16` private route on `dev ppp0`. On Home Wi-Fi, the more specific `/24` LAN route on the Wi-Fi device wins. Off Home Wi-Fi, `/24` disappears while all 5 CIFS shares stay mounted in `/proc/mounts`; any `stat()` on `$MEDIA_MOUNT_ROOT/$USER/...` sends SMB packets into `ppp0` and freezes the calling process.
2. **o2switch SSH Port 22 Whitelist on Mobile CGNAT**:
   cPanel *"Autorisation SSH"* allows only static whitelisted IPs (up to 5 slots or a DynDNS hostname refreshed every 3-5 minutes). Mobile 4G/5G CGNAT IPs time out on port `22`, whereas `HTTPS :443` with `X-API-Key` (`pass show services/pydbsrt/o2switch_api_key`) and `User-Agent: dbsrt-agent/0.1.0` works everywhere without tripping Tiger Protect WAF, including unit binary downloads via `GET /api/artifact/download?kind=phash|ahash|couplings&file=<basename>` (used automatically by `dbsrt_data.py fetch` when SSH is unreachable).
3. **Three Freebox Disks (`Disque_1`, `Disque_2`, `Disque_1_1`)**:
   - `Disque 1` (`Disque_1/Téléchargements`)
   - `Disque 2` (`Disque_2/Téléchargements`)
   - `Disque 1 1` (`Disque_1_1/Vidéos`) — mounted on workstation and RPi4 (`/mnt/freebox/Disque_1_1`), served by `scripts/infra/rpi4/rpi4_freebox_gateway.py`, and unified on SSOT in `scripts/portal_server.py`.
4. **Asymmetric Corpus Distribution & Hex Moviehash Sidecars (`local` vs `rpi4` vs `o2switch`)**:
   - `data/__AHASH__/` & `data/__PHASH__/`: Canonical fingerprints use 16-hex OpenSubtitles moviehash filenames (`<moviehash>.86.1fps.ahash`, `<moviehash>.24.0fps.phash`) paired with `<moviehash>.json` sidecars holding the human `media_name`. `dbsrt_data.py query ahash:<title>` and `phash:<title>` index all sidecars (accent-folded) with O(1) mtime caching and fall back to `o2switch` HTTPS `/api/assets` when missing locally. Cloud `o2switch` is the primary `.ahash` store (`rpi4` holds `0` `.ahash` files).
   - `data/catalog.dbphash` & `data/catalog.dbahash`: `catalog.dbphash` (`.dbphash` v2 visual index) is compiled on `o2switch` and mirrored to `rpi4`; `catalog.dbahash` (`.dbahash` v1 `mmap` acoustic archive) is mirrored on `o2switch` (`rpi4` holds `0` acoustic archives). Even on 4G/5G where SSH `:22` is filtered, `dbsrt_data.py` checks `o2switch` visual catalog freshness over HTTPS `:443` (`/api/phashes?limit=1`), and inspects `catalog.dbahash` `media_count` by reading only the 20-byte header prefix (`0` cold page faults).
   - `data/SRT/couplings.json`: Indexes full reference episodes/movies, NOT `*_mid15s.mp4` snippet filenames directly; `dbsrt_data.py` strips snippet/FPS suffixes automatically and cross-references `.phash`, `.ahash`, and `couplings.json` on every container search (`freebox:<q>`, `video:<q>`, `tvshow:<q>`) and `inspect`.
5. **Unseeded Isolated `git worktree`**:
   Binary datasets under `data/` and `.env` are `.gitignore`d. A fresh `git worktree` has an almost empty `data/` folder. `dbsrt_data.py` automatically resolves `git rev-parse --git-common-dir` to read the parent checkout's `.env` and `data/`, and provides `seed-worktree` to create all 8 dataset symlinks (`catalog.dbphash`, `catalog.dbahash`, `snippets`, `snippets_ahash`, `__PHASH__`, `__AHASH__`, `reports`, `SRT/couplings.json`) before starting daemons.

## 3. Extending the SSOT Registry (Adding a New Support or Dataset)

To register a new storage volume, cloud bucket, or artifact directory:
1. Add a declarative entry to `families` (or `freebox_disks`) in `scripts/data_access/sources_registry.json` with:
   - `id` (kebab-case), `category`, `name`, `desync_tier` (`immutable_unit`, `evolving_corpus`, or `live_state`), and `channels` ordered from fastest/safest to fallback.
   - Use runtime placeholders (`{local_media_root}`, `{effective_data_dir}`, `{repo_root}`, `{nas_lan_host}`, `{nas_wan_host}`, `{nas_user}`, `{rpi4_lan_host}`, `{o2switch_url}`) — **never** commit machine-local `$HOME/...` paths or credentials.
2. Optionally add a shorthand alias in `FAMILY_ALIASES` inside `scripts/data_access/resolver.py`.
3. Validate offline in `<50 ms`:
   ```bash
   python3 .agents/skills/pydbsrt-data-access/scripts/dbsrt_data.py --check
   task skills:validate
   ```
