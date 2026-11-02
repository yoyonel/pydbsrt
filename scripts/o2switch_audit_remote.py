"""Runs ON o2switch (cwd ~/dbsrt, /opt/alt/python313, piped by scripts/o2switch_audit.py): catalog self-recognition.

argv: offset, limit, seconds, cuts. For each .phash already in the catalog (sorted), send `cuts` windows of
`seconds` (spread over the media, 1 cut = the middle) to the prod daemon over the internal socket (/v1/recognize), and print one JSON row: answered
media, whether the title and season/episode match the file name, and the request time.
Rows contain media names: the local side writes them to a gitignored file and only prints counts.
"""

import json
import os
import re
import struct
import subprocess
import sys
import time

SOCK = "run/dbsrt-server.sock"
PHASH_DIR = "data/__PHASH__"
CATALOG = "data/catalog.dbphash"
PHASH_NAME = re.compile(r"^(.*)\.([0-9]+\.[0-9]+)fps\.phash$")
SEASON_EPISODE = re.compile(r"[Ss]([0-9]{1,2})[ ._-]?[Ee]([0-9]{1,3})")


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


def main():
    offset, limit, seconds, cuts = int(sys.argv[1]), int(sys.argv[2]), float(sys.argv[3]), int(sys.argv[4])
    key = open("etc/api_key.secret").read().strip()
    (catalog_media,) = struct.unpack_from("<I", open(CATALOG, "rb").read(16), 12)
    cat_mtime = os.path.getmtime(CATALOG)
    # Only files older than the catalog: a newer .phash is not ingested yet and cannot be recognized.
    files = sorted(
        f
        for f in os.listdir(PHASH_DIR)
        if PHASH_NAME.match(f) and os.path.getmtime(os.path.join(PHASH_DIR, f)) < cat_mtime
    )
    if offset == 0:
        print(json.dumps({"_meta": True, "eligible": len(files), "catalog_media": catalog_media}), flush=True)
    for name in files[offset : offset + limit]:
        # One write per file: an interrupted run never leaves a file with only some of its cuts.
        print("\n".join(json.dumps(r) for r in audit_file(name, seconds, key, cuts)), flush=True)


if __name__ == "__main__":
    main()
