"""Catalog self-recognition audit, run ON o2switch (cwd ~/dbsrt, ./bin/python3 = 3.13) by o2switch_ctl.sh audit.

`start RUN SECONDS CUTS` detaches the audit from the SSH session (fork + setsid) and returns: the workstation can
disconnect. For each .phash already in the catalog it sends CUTS windows of SECONDS to the prod daemon over the
internal socket (/v1/recognize) and appends one row per window to audit/RUN.jsonl. A rerun skips the files already
done. A lock (run/audit.lock, released by the kernel when the process dies) forbids two audits at once.
`status`, `stop` and `report [RUN]` print counts only: rows hold media names and never leave o2switch.
"""

import argparse
import datetime
import fcntl
import json
import os
import re
import signal
import struct
import subprocess
import sys
import time

SOCK = "run/dbsrt-server.sock"
PHASH_DIR = "data/__PHASH__"
CATALOG = "data/catalog.dbphash"
KEY_FILE = "etc/api_key.secret"
AUDIT_DIR = "audit"
LOCK = "run/audit.lock"
STATE = "run/audit.json"
LOG = "logs/audit.log"
PROGRESS_EVERY = 50
PHASH_NAME = re.compile(r"^(.*)\.([0-9]+\.[0-9]+)fps\.phash$")
SEASON_EPISODE = re.compile(r"[Ss]([0-9]{1,2})[ ._-]?[Ee]([0-9]{1,3})")
RUN_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]*")
DISTINCT_BINS = (0.0, 0.05, 0.10, 0.25, 0.50, 1.01)
MOVIEHASH = re.compile(r"[0-9a-fA-F]{16}")


def logical_name(directory, name):
    """Media name of a <moviehash>.<fps>fps.phash from its <moviehash>.json sidecar (scripts/phash_names.py rule)."""
    head, _, rest = name.partition(".")
    if len(head) != 16 or any(c not in "0123456789abcdefABCDEF" for c in head):
        return name
    try:
        media_name = json.load(open(os.path.join(directory, head + ".json"), encoding="utf-8")).get("media_name")
    except (OSError, ValueError, AttributeError):
        return name
    return media_name + "." + rest if isinstance(media_name, str) and media_name else name


def words(s):
    return re.findall(r"[^\W_]+", s.lower())


def contains_run(haystack, needle):
    n = len(needle)
    return n > 0 and any(haystack[i : i + n] == needle for i in range(len(haystack) - n + 1))


def title_matches(match, stem):
    """The answered title is right when one name's words appear, in order, inside the other's."""
    return contains_run(words(stem), words(match)) or contains_run(words(match), words(stem))


def season_episode(stem):
    se = SEASON_EPISODE.search(stem)
    return [int(se.group(1)), int(se.group(2))] if se else None


def cut_positions(cuts):
    """`cuts` windows evenly spread over the media; 1 cut is the middle."""
    return [(i + 0.5) / cuts for i in range(cuts)]


def window_at(hashes, fps, seconds, pos):
    n = int(fps * seconds)
    start = min(max(0, int(len(hashes) * pos) - n // 2), max(0, len(hashes) - n))
    return hashes[start : start + n]


def recognize(payload, key):
    """Up to 3 tries: an empty reply means the daemon was restarting or busy."""
    for _ in range(3):
        r = subprocess.run(
            [
                "curl",
                "-s",
                "-m",
                "60",
                "-K",
                "-",
                "--unix-socket",
                SOCK,
                "-H",
                "content-type: application/json",
                "--data-binary",
                json.dumps(payload),
                "http://localhost/v1/recognize",
            ],
            input=f'header = "x-api-key: {key}"\n',
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
        )
        try:
            resp = json.loads(r.stdout)
        except ValueError:
            resp = {}
        if resp:
            return resp
        time.sleep(15)
    return {}


def audit_file(name, seconds, key, cuts):
    # The file name is a moviehash after o2switch:phash:migrate: the truth is the sidecar's media name.
    m = PHASH_NAME.match(logical_name(PHASH_DIR, name))
    assert m, name
    stem, fps = m.groups()
    raw = open(os.path.join(PHASH_DIR, name), "rb").read()
    hashes = [int.from_bytes(raw[i : i + 8], "big") for i in range(0, len(raw) - 7, 8)]
    return [
        dict(audit_cut(stem, float(fps), window_at(hashes, float(fps), seconds, pos), pos, key), file=name)
        for pos in cut_positions(cuts)
    ]


def audit_cut(stem, fps, qs, pos, key):
    payload = {"query_fingerprint": [f"0x{h:016x}" for h in qs], "fps": fps, "language": "fr"}
    t0 = time.time()
    resp = recognize(payload, key)
    match = resp.get("media_match") or ""
    return {
        "stem": stem,
        "cut": pos,
        "match": match,
        "found": resp.get("found"),
        "conf": resp.get("confidence"),
        "http_ok": bool(resp),
        "frames": len(qs),
        "distinct": len(set(qs)),
        "right": title_matches(match, stem) if match else False,
        "stem_se": season_episode(stem),
        "resp_se": [resp.get("season"), resp.get("episode")],
        "n_cands": len(resp.get("candidates") or []),
        "tier": resp.get("tier"),
        "ms": int((time.time() - t0) * 1000),
    }


def run_file(run):
    """audit/RUN.jsonl; RUN comes from the command line, so it must stay a plain file name."""
    if not RUN_NAME.fullmatch(run or ""):
        sys.exit(f"invalid run name {run!r}: letters, digits, '_', '-', '.' only")
    return os.path.join(AUDIT_DIR, run + ".jsonl")


def take_lock(path):
    """An exclusive lock fd, or None when another process holds it. The kernel drops it when its holder dies."""
    fd = os.open(path, os.O_RDWR | os.O_CREAT, 0o600)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        os.close(fd)
        return None
    return fd


def load(path):
    meta, rows = None, []
    if os.path.exists(path):
        with open(path, encoding="utf-8") as fh:
            for line in fh:
                try:
                    row = json.loads(line)
                except ValueError:
                    continue  # a line cut by a kill: its file is not done and is audited again
                if row.get("_meta"):
                    meta = row
                else:
                    rows.append(row)
    return meta, rows


def eligible_files():
    # Only files older than the catalog: a newer .phash is not ingested yet and cannot be recognized.
    cat_mtime = os.path.getmtime(CATALOG)
    return sorted(
        f
        for f in os.listdir(PHASH_DIR)
        if PHASH_NAME.match(f) and os.path.getmtime(os.path.join(PHASH_DIR, f)) < cat_mtime
    )


def ends_with_newline(path):
    if not os.path.exists(path) or not os.path.getsize(path):
        return True
    with open(path, "rb") as fh:
        fh.seek(-1, os.SEEK_END)
        return fh.read(1) == b"\n"


def now():
    return datetime.datetime.now(datetime.UTC).strftime("%Y-%m-%dT%H:%M:%SZ")


def audit_run(run, seconds, cuts, key):
    path = run_file(run)
    files = eligible_files()
    _, rows = load(path)
    done = {r.get("file") for r in rows}
    with open(CATALOG, "rb") as fh:
        (catalog_media,) = struct.unpack_from("<I", fh.read(16), 12)
    with open(path, "a", encoding="utf-8") as out:
        if not ends_with_newline(path):
            out.write("\n")  # a kill cut the last line: keep the next row on its own line
        meta = {"_meta": True, "eligible": len(files), "catalog_media": catalog_media, "seconds": seconds, "cuts": cuts}
        out.write(json.dumps(meta) + "\n")
        out.flush()
        todo = [f for f in files if f not in done]
        print(f"{now()} start run={run} eligible={len(files)} done={len(files) - len(todo)} cuts={cuts}", flush=True)
        for i, name in enumerate(todo, 1):
            # One write per file: an interrupted run never leaves a file with only some of its cuts.
            out.write("".join(json.dumps(r) + "\n" for r in audit_file(name, seconds, key, cuts)))
            out.flush()
            if i % PROGRESS_EVERY == 0:
                print(f"{now()} progress files={len(files) - len(todo) + i}/{len(files)}", flush=True)
    print(f"{now()} finished run={run} files={len(files)}/{len(files)}", flush=True)
    return 0


def start(run, seconds, cuts):
    path = run_file(run)
    lock = take_lock(LOCK)
    if lock is None:
        print(f"Audit already running: {json.dumps(read_state())}")
        return 1
    if not os.path.exists(SOCK):
        print(f"Daemon socket {SOCK} missing: start dbsrt-server first.")
        return 1
    with open(KEY_FILE, encoding="utf-8") as fh:
        key = fh.read().strip()
    os.makedirs(os.path.dirname(path), mode=0o700, exist_ok=True)
    sys.stdout.flush()
    pid = os.fork()
    if pid:
        with open(STATE, "w", encoding="utf-8") as fh:
            json.dump({"pid": pid, "run": run, "seconds": seconds, "cuts": cuts, "started": now()}, fh)
        print(f"Audit started: PID {pid}, run={run}, rows in ~/dbsrt/{path}, log in ~/dbsrt/{LOG}")
        return 0
    # Child: own session, no fd left on the SSH channel, so the SSH command returns and a hangup cannot reach it.
    os.setsid()
    null = os.open(os.devnull, os.O_RDONLY)
    log = os.open(LOG, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
    os.dup2(null, 0)
    os.dup2(log, 1)
    os.dup2(log, 2)
    rc = 1
    try:
        rc = audit_run(run, seconds, cuts, key)
    except BaseException as exc:  # noqa: BLE001 - the log is the only witness of a detached process
        print(f"{now()} failed run={run}: {type(exc).__name__}: {exc}", flush=True)
    finally:
        os._exit(rc)


def read_state():
    try:
        with open(STATE, encoding="utf-8") as fh:
            return json.load(fh)
    except (OSError, ValueError):
        return {}


def is_running():
    lock = take_lock(LOCK)
    if lock is None:
        return True
    os.close(lock)
    return False


def status():
    state = read_state()
    running = is_running()
    print(
        f"{'RUNNING' if running else 'not running'} pid={state.get('pid')} run={state.get('run')} "
        f"cuts={state.get('cuts')} seconds={state.get('seconds')} started={state.get('started')}"
    )
    if state.get("run"):
        meta, rows = load(run_file(state["run"]))
        print(f"files={len({r.get('file') for r in rows})}/{meta['eligible'] if meta else '?'} rows={len(rows)}")
    return 0


def stop():
    if not is_running():
        print("Audit not running.")
        return 0
    pid = read_state().get("pid")
    cmdline = b""
    if isinstance(pid, int):
        try:
            with open(f"/proc/{pid}/cmdline", "rb") as fh:
                cmdline = fh.read()
        except OSError:
            pass
    if not isinstance(pid, int) or b"o2switch_audit_remote.py" not in cmdline:
        print(f"PID {pid} is not the audit: refusing to signal it.")
        return 1
    os.kill(pid, signal.SIGTERM)
    print(f"Sent SIGTERM to audit PID {pid}; a later start resumes from the rows already written.")
    return 0


def pct(n, d):
    return f"{100 * n / d:.1f}%" if d else "n/a"


def row_key(r):
    return r.get("file", r["stem"]), r.get("cut")


def report(path):
    meta, all_rows = load(path)
    rows = list({row_key(r): r for r in all_rows if r["http_ok"]}.values())
    n, right = len(rows), sum(r["right"] for r in rows)
    no_reply = len({row_key(r) for r in all_rows}) - n
    print(f"eligible={meta['eligible'] if meta else '?'} answered={n} no_reply={no_reply}")
    print(f"title_right={right} ({pct(right, n)}) empty_answer={sum(not r['match'] for r in rows)}")
    eps = [r for r in rows if r["stem_se"]]
    eps_title = [r for r in eps if r["right"]]
    exact = sum(r["resp_se"] == r["stem_se"] for r in eps_title)
    no_se = sum(r["resp_se"] == [None, None] for r in eps_title)
    print(f"episodes={len(eps)} title_right={len(eps_title)} episode_right={exact} ({pct(exact, len(eps))})")
    print(f"  title right, other episode={len(eps_title) - exact - no_se} title right, no season/episode={no_se}")
    movies = [r for r in rows if not r["stem_se"]]
    movies_right = sum(r["right"] for r in movies)
    print(f"movies={len(movies)} title_right={movies_right} ({pct(movies_right, len(movies))})")
    report_distinct_bins(rows)
    report_cuts(rows)
    ms = [r["ms"] for r in rows if "ms" in r]
    if ms:
        print(f"request_ms mean={sum(ms) / len(ms):.0f} max={max(ms)} total_min={sum(ms) / 60000:.1f}")
    return 0


def report_cuts(rows):
    for cut in sorted({r.get("cut") or 0.5 for r in rows}):
        b = [r for r in rows if (r.get("cut") or 0.5) == cut]
        right = sum(r["right"] for r in b)
        print(f"cut={cut:.2f} n={len(b)} title_right={right} ({pct(right, len(b))})")
    by_file = {}
    for r in rows:
        by_file.setdefault(r.get("file", r["stem"]), []).append(r["right"])
    print(
        f"files={len(by_file)} any_cut_right={sum(any(v) for v in by_file.values())}"
        f" all_cuts_right={sum(all(v) for v in by_file.values())}"
        f" moviehash_without_sidecar={len({r['stem'] for r in rows if MOVIEHASH.fullmatch(r['stem'])})}"
    )


def report_distinct_bins(rows):
    print("distinct/frames bin: n wrong%")
    for lo, hi in zip(DISTINCT_BINS, DISTINCT_BINS[1:]):
        b = [r for r in rows if lo <= r["distinct"] / max(r["frames"], 1) < hi]
        print(f"  [{lo:.2f},{hi:.2f}) n={len(b):5d} wrong={pct(sum(not r['right'] for r in b), len(b))}")


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("start")
    s.add_argument("run")
    s.add_argument("seconds", type=float)
    s.add_argument("cuts", type=int)
    sub.add_parser("status")
    sub.add_parser("stop")
    sub.add_parser("report").add_argument("run", nargs="?", help="default: the last started run")
    a = p.parse_args()
    if a.cmd == "start":
        return start(a.run, a.seconds, a.cuts)
    if a.cmd == "report":
        return report(run_file(a.run or read_state().get("run")))
    return status() if a.cmd == "status" else stop()


if __name__ == "__main__":
    sys.exit(main())
