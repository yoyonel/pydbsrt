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
import urllib.request
from collections.abc import Callable
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
CTL = REPO_ROOT / "scripts" / "o2switch_ctl.sh"
HOST = os.environ.get("O2SWITCH_HOST", "o2switch")
REMOTE_BASE = os.environ.get("REMOTE_BASE", "dbsrt")
REMOTE_HELPER = f"{REMOTE_BASE}/bin/dbsrt_server_ctl.sh"
STOP_MARKER = f"{REMOTE_BASE}/run/dbsrt-server.stopped"
API_KEY_PASS_ENTRY = os.environ.get("API_KEY_PASS_ENTRY", "services/pydbsrt/o2switch_api_key")
WATCHDOG_TAG = "# dbsrt-server watchdog"
PACK_TAG = "# dbsrt-server catalog pack"
# Never fires (31 February): only trips the legacy-watchdog guard of stop
LEGACY_WATCHDOG_LINE = f"0 0 31 2 * true {WATCHDOG_TAG}"
RECOGNIZE_SAMPLES = 5
QUERY_SECONDS = 10
HTTP_TIMEOUT_SEC = 60
CTL_TIMEOUT_SEC = 600
PID_POLL_TIMEOUT_SEC = 10.0
PID_POLL_INTERVAL_SEC = 0.5

# Runs remotely from REMOTE_BASE: one /v1/recognize payload per line, cut mid-media from a random
# .phash already packed in the catalog. `_stem` is the file stem, kept local for right_media.
PICK_QUERIES = r"""
import json, os, random, re, sys
count, seconds = int(sys.argv[1]), float(sys.argv[2])
d = "data/__PHASH__"
cat_mtime = os.path.getmtime("data/catalog.dbphash")
pat = re.compile(r"^(.*)\.([0-9]+\.[0-9]+)fps\.phash$")
files = [f for f in os.listdir(d) if pat.match(f) and os.path.getmtime(os.path.join(d, f)) < cat_mtime]
for f in random.sample(files, min(count, len(files))):
    stem, fps = pat.match(f).groups()
    raw = open(os.path.join(d, f), "rb").read()
    hashes = [int.from_bytes(raw[i : i + 8], "big") for i in range(0, len(raw) - 7, 8)]
    n = int(float(fps) * seconds)
    start = max(0, len(hashes) // 2 - n // 2)
    query = ["0x%016x" % h for h in hashes[start : start + n]]
    print(json.dumps({"query_fingerprint": query, "fps": float(fps), "language": "fr", "_stem": stem}))
"""


def words(s: str) -> list[str]:
    return re.findall(r"[^\W_]+", s.lower())


def contains_run(haystack: list[str], needle: list[str]) -> bool:
    n = len(needle)
    return n > 0 and any(haystack[i : i + n] == needle for i in range(len(haystack) - n + 1))


def right_media(media_match: str, stem: str) -> bool:
    """media_match is either a clean title (shorter than the stem) or the catalog file name (stem plus
    its fps suffix): one word run must contain the other. Whole words, so "Le Film 2" does not match
    "Le.Film.2010".

    shortcut: a clean series title also passes for another episode, it does not name the episode.
    """
    match_words, stem_words = words(media_match), words(stem)
    return contains_run(stem_words, match_words) or contains_run(match_words, stem_words)


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

    def pids(self) -> list[int]:
        return sorted(int(p) for p in self.ssh(f"bash {REMOTE_HELPER} pids {REMOTE_BASE}").stdout.split())

    def wait_pids(self, count: int) -> list[int]:
        deadline = time.monotonic() + PID_POLL_TIMEOUT_SEC
        while len(pids := self.pids()) != count and time.monotonic() < deadline:
            time.sleep(PID_POLL_INTERVAL_SEC)
        return pids

    def lock_holders(self) -> int:
        """Processes with an open fd on the spawn lock: only a spawner, while it spawns."""
        out = self.ssh(f"ls -l /proc/[0-9]*/fd 2>/dev/null | grep -c {REMOTE_BASE}/run/gateway_spawn.lock")
        return int(out.stdout.strip() or 0)

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

    def kill(self, pids: list[int]) -> None:
        self.ssh("kill -9 " + " ".join(map(str, pids)))
        self.check("kill -9: 0 daemon", self.wait_pids(0) == [])

    def recognize(self) -> None:
        lines = self.ssh(f"cd {REMOTE_BASE} && python3 -I - {RECOGNIZE_SAMPLES} {QUERY_SECONDS}", PICK_QUERIES)
        payloads = [json.loads(line) for line in lines.stdout.splitlines()]
        self.check(f"picked {RECOGNIZE_SAMPLES} catalog queries", len(payloads) == RECOGNIZE_SAMPLES)
        found = right = timed = 0
        for payload in payloads:
            stem = payload.pop("_stem")
            code, body = self.http("/v1/recognize", payload=payload)
            resp = json.loads(body) if code == 200 else {}
            found += bool(resp.get("found"))
            right += right_media(resp.get("media_match") or "", stem)
            timed += (resp.get("timer_seconds") or 0) > 0
        total = len(payloads)
        self.check(f"recognize found: {found}/{total}", total > 0 and found == total)
        self.check(f"recognize right_media: {right}/{total}", total > 0 and right == total)
        self.check(f"recognize timer_seconds > 0: {timed}/{total}", total > 0 and timed == total)

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
