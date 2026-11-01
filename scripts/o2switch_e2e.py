#!/usr/bin/env python3
"""E2E recette of dbsrt-server on o2switch: lifecycle, respawn paths, cron and public gateway.

Drives scripts/o2switch_ctl.sh against the live host and prints one PASS/FAIL line per check.
Command outputs are captured, never printed: no media name, API key or remote path leaks.
Always ends with the cron installed and the daemon started.
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from collections.abc import Callable, Iterator
from contextlib import contextmanager
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
CTL = REPO_ROOT / "scripts" / "o2switch_ctl.sh"
HOST = os.environ.get("O2SWITCH_HOST", "o2switch")
REMOTE_BASE = os.environ.get("REMOTE_BASE", "dbsrt")
REMOTE_HELPER = f"{REMOTE_BASE}/bin/dbsrt_server_ctl.sh"
RELEASE_HELPER = f"{REMOTE_BASE}/bin/dbsrt_release.sh"
STOP_MARKER = f"{REMOTE_BASE}/run/dbsrt-server.stopped"
# Serializes every spawn: PHP gateway, cron watchdog and ctl start/stop
SPAWN_LOCK = f"{REMOTE_BASE}/run/gateway_spawn.lock"
SPAWN_LOCK_WAIT_SEC = 30.0
# The holder releases the lock on its own if this process dies before closing its stdin
SPAWN_LOCK_HOLD_SEC = 60
API_KEY_PASS_ENTRY = os.environ.get("API_KEY_PASS_ENTRY", "services/pydbsrt/o2switch_api_key")
WATCHDOG_TAG = "# dbsrt-server watchdog"
PACK_TAG = "# dbsrt-server catalog pack"
# Never fires (31 February): only trips the legacy-watchdog guard of stop
LEGACY_WATCHDOG_LINE = f"0 0 31 2 * true {WATCHDOG_TAG}"
GOLDEN_FILE = f"{REMOTE_BASE}/etc/e2e_golden.json"
GOLDEN_PLAIN = 10
GOLDEN_COUPLED = 5
GOLDEN_SIZE = GOLDEN_PLAIN + GOLDEN_COUPLED
# Candidates cut per kept query when (re)building the fixed set: some known-hard queries fail
GOLDEN_POOL_FACTOR = 4
QUERY_SECONDS = 10
HTTP_TIMEOUT_SEC = 60
CTL_TIMEOUT_SEC = 600
PID_POLL_TIMEOUT_SEC = 10.0
PID_POLL_INTERVAL_SEC = 0.5

# Runs remotely from REMOTE_BASE (Python 3.6): one /v1/recognize payload per line, cut mid-media.
# `golden SECONDS`: the fixed set of etc/e2e_golden.json, `{"_missing": true}` for a vanished file.
# `pool SECONDS PLAIN COUPLED`: the first .phash already packed in the catalog, in sha256 order of
# their name, split by French certified coupling. The fixed set stays on o2switch: no media name in git.
# `_stem`, `_file` and `_coupled` stay local; media names are never printed.
CUT_QUERIES = r"""
import hashlib, json, os, re, sys
mode, seconds = sys.argv[1], float(sys.argv[2])
d = "data/__PHASH__"
pat = re.compile(r"^(.*)\.([0-9]+\.[0-9]+)fps\.phash$")


def cut(f, coupled, stem=None):
    stem, fps = stem or pat.match(f).group(1), pat.match(f).group(2)
    raw = open(os.path.join(d, f), "rb").read()
    hashes = [int.from_bytes(raw[i : i + 8], "big") for i in range(0, len(raw) - 7, 8)]
    n = int(float(fps) * seconds)
    start = max(0, len(hashes) // 2 - n // 2)
    query = ["0x%016x" % h for h in hashes[start : start + n]]
    print(json.dumps({"query_fingerprint": query, "fps": float(fps), "language": "fr", "_stem": stem,
                      "_file": f, "_coupled": coupled}))


def moviehash_twin(stem):
    # The RPi4 dedupe deletes a legacy .phash identical to its `<moviehash>.<fps>fps.phash` twin,
    # whose `<moviehash>.json` sidecar keeps the media name
    for f in os.listdir(d):
        m = pat.match(f)
        side = os.path.join(d, f.split(".")[0] + ".json")
        if m and re.match(r"^[0-9a-f]{16}$", m.group(1)) and os.path.isfile(side):
            name = json.load(open(side)).get("media_name", "")
            if stem in (name, os.path.splitext(name)[0]):
                return f
    return None


if mode == "golden":
    for entry in json.load(open("etc/e2e_golden.json")):
        m = pat.match(entry["file"])
        twin = m and not os.path.isfile(os.path.join(d, entry["file"])) and moviehash_twin(m.group(1))
        if twin:
            cut(twin, entry["coupled"], m.group(1))
        elif m and os.path.isfile(os.path.join(d, entry["file"])):
            cut(entry["file"], entry["coupled"])
        else:
            print(json.dumps({"_missing": True}))
else:
    plain, coupled = int(sys.argv[3]), int(sys.argv[4])
    cat_mtime = os.path.getmtime("data/catalog.dbphash")
    couplings = json.load(open("data/SRT/couplings.json")).get("couplings", [])
    french = {c.get("phash_filename") for c in couplings
              if any(str(s.get("lang", "")).lower() in ("fr", "fre", "fra", "french") for s in c.get("subtitles", []))}
    files = sorted((f for f in os.listdir(d) if pat.match(f) and os.path.getmtime(os.path.join(d, f)) < cat_mtime),
                   key=lambda f: hashlib.sha256(f.encode()).hexdigest())
    for f in [f for f in files if f not in french][:plain] + [f for f in files if f in french][:coupled]:
        cut(f, f in french)
"""
SXXEYY = re.compile(r"[sS](\d{1,2})[eE](\d{1,3})")


def words(s: str) -> list[str]:
    return re.findall(r"[^\W_]+", s.lower())


def contains_run(haystack: list[str], needle: list[str]) -> bool:
    n = len(needle)
    return n > 0 and any(haystack[i : i + n] == needle for i in range(len(haystack) - n + 1))


def right_media(media_match: str, stem: str) -> bool:
    """media_match is either a clean title (shorter than the stem) or the catalog file name (stem plus
    its fps suffix): one word run must contain the other. Whole words, so "Le Film 2" does not match
    "Le.Film.2010".

    The stem is percent-decoded like MediaMetadata::parse: catalog file names may carry `%20`.

    shortcut: a clean series title also passes for another episode, it does not name the episode.
    """
    match_words, stem_words = words(media_match), words(urllib.parse.unquote(stem))
    return contains_run(stem_words, match_words) or contains_run(match_words, stem_words)


def golden_ok(resp: dict, stem: str, coupled: bool) -> bool:
    """A fixed query passes when it names its own media, its own SxxEyy (episode stems) and, when the
    media has a French certified coupling, offers only certified subtitles of its own release."""
    if not resp.get("found") or not right_media(resp.get("media_match") or "", stem):
        return False
    decoded = urllib.parse.unquote(stem)
    sxe = SXXEYY.search(decoded)
    if sxe and (resp.get("season"), resp.get("episode")) != (int(sxe[1]), int(sxe[2])):
        return False
    cands = resp.get("candidates") or []
    return not coupled or (
        bool(cands) and all(urllib.parse.unquote(str(c.get("release") or "")).startswith(decoded) for c in cands)
    )


def pick_golden(results: Iterator[tuple[dict, bool]], plain: int, coupled: int) -> list[dict]:
    """First passing queries of each kind, in pool order; stops reading the pool once the set is full."""
    wanted = {False: plain, True: coupled}
    kept: list[dict] = []
    for query, ok in results:
        if ok and wanted[query["_coupled"]] > 0:
            wanted[query["_coupled"]] -= 1
            kept.append({"file": query["_file"], "coupled": query["_coupled"]})
        if not any(wanted.values()):
            break
    return kept


def parse_releases(listing: str) -> tuple[str | None, str | None]:
    """(current, previous) release ids from `dbsrt_release.sh list` ("* id build", "- id build")."""
    marked = {line[0]: line.split()[1] for line in listing.splitlines() if line[:2] in ("* ", "- ")}
    return marked.get("*"), marked.get("-")


class Recette:
    def __init__(self, url: str, key: str) -> None:
        self.url = url
        self.key = key
        self.failed = 0

    def check(self, name: str, ok: bool) -> bool:
        self.failed += not ok
        print(f"{'PASS' if ok else 'FAIL'}  {name}", flush=True)
        return ok

    def ctl(self, action: str) -> int:
        env = {**os.environ, "O2SWITCH_URL": self.url}
        return subprocess.run([str(CTL), action], env=env, capture_output=True, timeout=CTL_TIMEOUT_SEC).returncode

    def ssh(self, cmd: str, stdin: str | None = None) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["ssh", "-o", "BatchMode=yes", HOST, cmd], input=stdin, capture_output=True, text=True, timeout=120
        )

    def hold(self, cmd: str) -> subprocess.Popen[str]:
        return subprocess.Popen(
            ["ssh", "-o", "BatchMode=yes", HOST, cmd], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True
        )

    @contextmanager
    def spawns_blocked(self) -> Iterator[bool]:
        """Holds the spawn lock: live traffic (an open monitoring page) cannot respawn the daemon meanwhile.

        Yields whether the lock was taken within SPAWN_LOCK_WAIT_SEC.
        """
        holder = self.hold(
            f"flock -w {SPAWN_LOCK_WAIT_SEC} {SPAWN_LOCK} -c 'echo locked; read -t {SPAWN_LOCK_HOLD_SEC} _'"
        )
        assert holder.stdin is not None and holder.stdout is not None
        try:
            yield holder.stdout.readline().strip() == "locked"
        finally:
            holder.stdin.close()
            holder.wait(timeout=SPAWN_LOCK_WAIT_SEC + 30)

    def pids(self) -> list[int]:
        return sorted(int(p) for p in self.ssh(f"bash {REMOTE_HELPER} pids {REMOTE_BASE}").stdout.split())

    def wait_until(self, done: Callable[[list[int]], bool]) -> list[int]:
        deadline = time.monotonic() + PID_POLL_TIMEOUT_SEC
        while not done(pids := self.pids()) and time.monotonic() < deadline:
            time.sleep(PID_POLL_INTERVAL_SEC)
        return pids

    def wait_pids(self, count: int) -> list[int]:
        return self.wait_until(lambda pids: len(pids) == count)

    def lock_holders(self) -> int:
        """Processes with an open fd on the spawn lock: only a spawner, while it spawns."""
        out = self.ssh(f"ls -l /proc/[0-9]*/fd 2>/dev/null | grep -c {REMOTE_BASE}/run/gateway_spawn.lock")
        return int(out.stdout.strip() or 0)

    def releases(self) -> tuple[str | None, str | None]:
        return parse_releases(self.ssh(f"bash {RELEASE_HELPER} list {REMOTE_BASE}").stdout)

    def running(self) -> list[str]:
        return self.ssh(f"bash {RELEASE_HELPER} running {REMOTE_BASE}").stdout.split()

    def marker(self) -> bool:
        return self.ssh(f"test -e {STOP_MARKER}").returncode == 0

    def cron_count(self, keep: Callable[[str], bool]) -> int:
        return sum(keep(line) for line in self.ssh("crontab -l").stdout.splitlines())

    def http(self, path: str, auth: bool = True, payload: dict | None = None) -> tuple[int, bytes]:
        headers = {"Content-Type": "application/json", **({"X-API-Key": self.key} if auth else {})}
        data = json.dumps(payload).encode() if payload is not None else None
        req = urllib.request.Request(self.url + path, data=data, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=HTTP_TIMEOUT_SEC) as resp:
                return resp.status, resp.read()
        except urllib.error.HTTPError as e:
            return e.code, b""

    def run(self) -> None:
        # First: stop refuses to run while a pre-helper watchdog line is installed
        print("== cron")
        self.cron()

        print("== start / status")
        self.check("start exits 0", self.ctl("start") == 0)
        first = self.pids()
        self.check("start: exactly 1 daemon, no stop marker", len(first) == 1 and not self.marker())
        self.check("start again: exits 0, same daemon", self.ctl("start") == 0 and self.pids() == first)
        self.check("status exits 0 (one daemon)", self.ctl("status") == 0)

        print("== gateway")
        self.check("health with key: 200", self.http("/v1/health")[0] == 200)
        self.check("health without key: 401", self.http("/v1/health", auth=False)[0] == 401)
        self.check("demo exits 0", self.ctl("demo") == 0)
        self.recognize()

        print("== stop blocks every respawn path")
        self.check("stop exits 0", self.ctl("stop") == 0)
        self.check("stop: 0 daemon, stop marker set", self.pids() == [] and self.marker())
        self.check("status exits 3 (stopped)", self.ctl("status") == 3)
        self.check("gateway request: 503", self.http("/v1/health")[0] == 503)
        self.check("gateway request: no respawn", self.pids() == [])
        self.ssh(f"bash {REMOTE_HELPER} watchdog {REMOTE_BASE}")
        self.check("watchdog: no respawn", self.wait_pids(1) == [])

        print("== start / restart")
        self.check("start after stop exits 0", self.ctl("start") == 0)
        before = self.pids()
        self.check("start: 1 daemon, marker cleared", len(before) == 1 and not self.marker())
        self.check("health after start: 200", self.http("/v1/health")[0] == 200)
        self.check("restart exits 0", self.ctl("restart") == 0)
        after = self.pids()
        self.check("restart: 1 new daemon", len(after) == 1 and after != before)
        self.check("helper spawn: no process inherits the spawn lock", self.lock_holders() == 0)

        print("== respawn after crash")
        with self.spawns_blocked() as locked:
            self.check("spawn lock held: no respawn between kill and status", locked)
            self.kill(after)
            self.check("status exits 3 (crashed, no marker)", self.ctl("status") == 3 and not self.marker())
        self.check("gateway respawns: health 200", self.http("/v1/health")[0] == 200)
        respawned = self.pids()
        self.check("gateway respawn: 1 new daemon", len(respawned) == 1 and respawned != after)
        self.check("gateway respawn: no process inherits the spawn lock", self.lock_holders() == 0)
        self.kill(respawned)
        self.ssh(f"bash {REMOTE_HELPER} watchdog {REMOTE_BASE}")
        watched = self.wait_pids(1)
        self.check("watchdog respawn: 1 new daemon", len(watched) == 1 and watched != respawned)

        print("== releases")
        self.rollback_round_trip()

    def rollback_round_trip(self) -> None:
        current, previous = self.releases()
        self.check("releases: daemon executes the current release", current is not None and self.running() == [current])
        if not self.check("releases: a previous release exists", previous is not None):
            return
        for target, other in ((previous, current), (current, previous)):
            self.check("rollback exits 0", self.ctl("rollback") == 0)
            self.check("rollback: current and previous swapped", self.releases() == (target, other))
            self.check("rollback: daemon executes the new current release", self.running() == [target])
            self.check("health after rollback: 200", self.http("/v1/health")[0] == 200)

    def kill(self, pids: list[int]) -> None:
        # Live traffic or the cron can respawn a daemon right away: only the killed pids must go.
        self.ssh("kill -9 " + " ".join(map(str, pids)))
        left = self.wait_until(lambda now: not set(now) & set(pids))
        self.check("kill -9: killed daemon gone", not set(left) & set(pids))

    def cut_queries(self, *args: object) -> list[dict]:
        out = self.ssh(f"cd {REMOTE_BASE} && python3 -I - {' '.join(map(str, args))}", CUT_QUERIES)
        return [json.loads(line) for line in out.stdout.splitlines()]

    def ask(self, query: dict) -> dict:
        code, body = self.http("/v1/recognize", payload={k: v for k, v in query.items() if not k.startswith("_")})
        return json.loads(body) if code == 200 else {}

    def build_golden(self) -> None:
        pool = self.cut_queries(
            "pool", QUERY_SECONDS, GOLDEN_PLAIN * GOLDEN_POOL_FACTOR, GOLDEN_COUPLED * GOLDEN_POOL_FACTOR
        )
        results = ((q, golden_ok(self.ask(q), q["_stem"], q["_coupled"])) for q in pool)
        kept = pick_golden(results, GOLDEN_PLAIN, GOLDEN_COUPLED)
        self.ssh(
            f"mkdir -p {REMOTE_BASE}/etc && cat > {GOLDEN_FILE}.tmp && mv -f {GOLDEN_FILE}.tmp {GOLDEN_FILE}",
            json.dumps(kept),
        )
        print(f"golden set built: {len(kept)}/{GOLDEN_SIZE} queries")

    def recognize(self) -> None:
        if self.ssh(f"test -s {GOLDEN_FILE}").returncode != 0:
            self.build_golden()
        queries = self.cut_queries("golden", QUERY_SECONDS)
        present = [q for q in queries if not q.get("_missing")]
        self.check(
            f"golden set: {len(present)}/{GOLDEN_SIZE} fixed queries", len(present) == GOLDEN_SIZE == len(queries)
        )
        passed = {False: 0, True: 0}
        timed = 0
        for query in present:
            resp = self.ask(query)
            passed[query["_coupled"]] += golden_ok(resp, query["_stem"], query["_coupled"])
            timed += (resp.get("timer_seconds") or 0) > 0
        plain = sum(not q["_coupled"] for q in present)
        coupled = len(present) - plain
        self.check(f"golden plain: {passed[False]}/{plain} own media and episode", plain > 0 and passed[False] == plain)
        self.check(
            f"golden coupled: {passed[True]}/{coupled} own certified subtitles", coupled > 0 and passed[True] == coupled
        )
        self.check(f"recognize timer_seconds > 0: {timed}/{len(present)}", present != [] and timed == len(present))

    def cron(self) -> None:
        def is_watchdog(line: str) -> bool:
            return WATCHDOG_TAG in line and "dbsrt_server_ctl.sh watchdog" in line

        self.check("setup-cron twice exits 0", self.ctl("setup-cron") == 0 and self.ctl("setup-cron") == 0)
        self.check("cron: 1 watchdog line on dbsrt_server_ctl.sh", self.cron_count(is_watchdog) == 1)
        self.check("cron: 1 catalog pack line", self.cron_count(lambda line: PACK_TAG in line) == 1)
        self.check("remove-cron exits 0", self.ctl("remove-cron") == 0)
        self.check(
            "remove-cron: 0 dbsrt line",
            self.cron_count(lambda line: WATCHDOG_TAG in line or PACK_TAG in line) == 0,
        )
        self.ssh(f"(crontab -l 2>/dev/null; echo '{LEGACY_WATCHDOG_LINE}') | crontab -")
        running = self.pids()
        self.check("stop refused under a legacy watchdog cron", self.ctl("stop") != 0 and self.pids() == running)
        self.check("setup-cron exits 0", self.ctl("setup-cron") == 0)
        self.check(
            "setup-cron replaces the legacy line",
            self.cron_count(lambda line: WATCHDOG_TAG in line) == 1 and self.cron_count(is_watchdog) == 1,
        )


def read_url() -> str:
    url = os.environ.get("O2SWITCH_URL", "")
    env_file = REPO_ROOT / ".env"
    if not url and env_file.is_file():
        for line in env_file.read_text().splitlines():
            if line.startswith("O2SWITCH_URL="):
                url = line.split("=", 1)[1].strip().strip("'\"")
    return url.rstrip("/")


def main() -> int:
    url = read_url()
    if not url:
        print("O2SWITCH_URL not set (environment or .env): the recette tests the public gateway.")
        return 2
    key = subprocess.run(["pass", "show", API_KEY_PASS_ENTRY], capture_output=True, text=True, check=True)
    recette = Recette(url, key.stdout.splitlines()[0])
    if "--reset-golden" in sys.argv[1:]:
        recette.ssh(f"rm -f {GOLDEN_FILE}")
    try:
        recette.run()
    finally:
        # Leaves production serving whatever failed
        recette.ctl("setup-cron")
        recette.ctl("start")
    print(f"== {'FAIL' if recette.failed else 'PASS'}: {recette.failed} failed check(s)")
    return 1 if recette.failed else 0


if __name__ == "__main__":
    sys.exit(main())
