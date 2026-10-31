import hashlib
import itertools
import json
import os
import subprocess
from collections.abc import Iterator
from pathlib import Path

import pytest

from scripts import o2switch_e2e
from scripts.o2switch_e2e import Recette, golden_ok, parse_releases, pick_golden, right_media


class FakeRecette(Recette):
    """Replays daemon pid listings; the last one repeats."""

    def __init__(self, listings: list[list[int]]) -> None:
        super().__init__("http://gateway", "key")
        self.listings = itertools.chain(listings, itertools.repeat(listings[-1]))

    def ssh(self, cmd: str, stdin: str | None = None) -> subprocess.CompletedProcess[str]:
        return subprocess.CompletedProcess([], 0, "", "")

    def pids(self) -> list[int]:
        return next(self.listings)


@pytest.mark.parametrize(
    ("listings", "ok"),
    [
        ([[]], True),
        ([[7]], True),
        ([[1], [1, 7], [7]], True),
        ([[1]], False),
        ([[1, 7]], False),
    ],
    ids=["none-left", "respawned-by-other", "gone-after-polls", "still-alive", "alive-with-respawn"],
)
def test_kill_checks_killed_pids_only(monkeypatch: pytest.MonkeyPatch, listings: list[list[int]], ok: bool) -> None:
    monkeypatch.setattr(o2switch_e2e, "PID_POLL_TIMEOUT_SEC", 0.2)
    monkeypatch.setattr(o2switch_e2e, "PID_POLL_INTERVAL_SEC", 0.0)
    recette = FakeRecette(listings)
    recette.kill([1])
    assert recette.failed == (0 if ok else 1)


class LocalHold(Recette):
    """Runs the lock holder command locally, as the remote shell would."""

    def __init__(self) -> None:
        super().__init__("http://gateway", "key")

    def hold(self, cmd: str) -> subprocess.Popen[str]:
        return subprocess.Popen(["sh", "-c", cmd], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)


def lock_free(lock: Path) -> bool:
    return subprocess.run(["flock", "-n", str(lock), "true"]).returncode == 0


@pytest.fixture
def spawn_lock(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> Path:
    lock = tmp_path / "gateway_spawn.lock"
    monkeypatch.setattr(o2switch_e2e, "SPAWN_LOCK", str(lock))
    monkeypatch.setattr(o2switch_e2e, "SPAWN_LOCK_WAIT_SEC", 0.2)
    return lock


def test_spawns_blocked_holds_the_spawn_lock_until_exit(spawn_lock: Path) -> None:
    with LocalHold().spawns_blocked() as locked:
        assert locked
        assert not lock_free(spawn_lock)
    assert lock_free(spawn_lock)


def test_spawns_blocked_reports_a_lock_held_elsewhere(spawn_lock: Path) -> None:
    other = LocalHold().hold(f"flock {spawn_lock} -c 'echo locked; read _'")
    assert other.stdout is not None and other.stdin is not None
    assert other.stdout.readline().strip() == "locked"
    try:
        with LocalHold().spawns_blocked() as locked:
            assert not locked
    finally:
        other.stdin.close()
        other.wait(timeout=5)


@pytest.mark.parametrize(
    ("media_match", "stem", "expected"),
    [
        ("Le Film", "Le.Film.2010.1080p.x264", True),
        ("Le.Film.2010.1080p.23.976fps", "Le.Film.2010.1080p", True),
        ("Le.Film.2010.1080p.23.976fps", "Autre.Film.2010.1080p", False),
        ("LE FILM", "le_film_2010", True),
        ("Show", "Show.S01E02.720p", True),
        ("Été", "Été.2019.1080p", True),
        ("Autre Film", "Le.Film.2010.1080p", False),
        ("", "Le.Film.2010", False),
        ("...", "Le.Film.2010", False),
        ("Le Film 2", "Le.Film.2010", False),
        ("Show Name", "Show%20Name%20S01E02%201080p", True),
        ("Autre Show", "Show%20Name%20S01E02", False),
    ],
    ids=[
        "dots",
        "file-name-match",
        "file-name-other",
        "case",
        "series-title",
        "accents",
        "other-media",
        "empty",
        "punctuation-only",
        "longer-title",
        "percent-encoded-stem",
        "percent-encoded-other",
    ],
)
def test_right_media(media_match: str, stem: str, expected: bool) -> None:
    assert right_media(media_match, stem) is expected


@pytest.mark.parametrize(
    ("listing", "expected"),
    [
        ("* b abc-dirty feat/x\n- a \n  c \n", ("b", "a")),
        ("* b \n", ("b", None)),
        ("  c \n", (None, None)),
        ("", (None, None)),
    ],
    ids=["current-previous-other", "first-deploy", "no-link", "no-release"],
)
def test_parse_releases(listing: str, expected: tuple[str | None, str | None]) -> None:
    assert parse_releases(listing) == expected


class FakeReleases(Recette):
    """Simulates dbsrt_release.sh rollback: swaps current/previous; the daemon follows unless stuck."""

    def __init__(self, current: str, previous: str | None, stuck: bool = False) -> None:
        super().__init__("http://gateway", "key")
        self.state = (current, previous)
        self.daemon = current
        self.stuck = stuck

    def releases(self) -> tuple[str | None, str | None]:
        return self.state

    def running(self) -> list[str]:
        return [self.daemon] if self.daemon else []

    def ctl(self, action: str) -> int:
        assert action == "rollback"
        self.state = (self.state[1], self.state[0])
        if not self.stuck:
            self.daemon = self.state[0]
        return 0

    def http(self, path: str, auth: bool = True, payload: dict | None = None) -> tuple[int, bytes]:
        return 200, b""


@pytest.mark.parametrize(
    ("previous", "stuck", "failed"),
    [("a", False, 0), (None, False, 1), ("a", True, 1)],
    ids=["round-trip", "no-previous", "daemon-not-restarted"],
)
def test_rollback_round_trip(previous: str | None, stuck: bool, failed: int) -> None:
    recette = FakeReleases("b", previous, stuck)
    recette.rollback_round_trip()
    assert recette.failed == failed
    assert recette.state == ("b", previous)


SHOW = "Show%20Name%20S01E02%201080p"
OWN = [{"source": "certified", "release": "Show Name S01E02 1080p.23.976fps.phash"}]
OTHER = [{"source": "certified", "release": "Show Name S01E03 1080p.23.976fps.phash"}]


def answer(
    match: str = "Show Name", season: int | None = 1, episode: int | None = 2, cands: list | None = None
) -> dict:
    return {"found": True, "media_match": match, "season": season, "episode": episode, "candidates": cands}


@pytest.mark.parametrize(
    ("resp", "stem", "coupled", "expected"),
    [
        (answer("Le Film", None, None), "Le.Film.2010.1080p", False, True),
        (answer("Autre Film", None, None), "Le.Film.2010.1080p", False, False),
        ({"found": False}, "Le.Film.2010.1080p", False, False),
        (answer(), SHOW, False, True),
        (answer(episode=3), SHOW, False, False),
        (answer(season=None, episode=None), SHOW, False, False),
        (answer(cands=OWN), SHOW, True, True),
        (answer(), SHOW, True, False),
        (answer(cands=OWN + OTHER), SHOW, True, False),
        (answer(cands=OTHER), SHOW, False, True),
    ],
    ids=[
        "movie-right",
        "movie-wrong",
        "not-found",
        "episode-right",
        "episode-wrong",
        "episode-missing",
        "coupled-own-subtitle",
        "coupled-no-subtitle",
        "coupled-foreign-subtitle",
        "plain-ignores-candidates",
    ],
)
def test_golden_ok(resp: dict, stem: str, coupled: bool, expected: bool) -> None:
    assert golden_ok(resp, stem, coupled) is expected


def query(stem: str, coupled: bool) -> dict:
    return {
        "query_fingerprint": ["0x1"],
        "fps": 25.0,
        "_stem": stem,
        "_file": f"{stem}.25.000fps.phash",
        "_coupled": coupled,
    }


def test_pick_golden_keeps_first_passing_per_kind() -> None:
    def results() -> Iterator[tuple[dict, bool]]:
        yield query("a", False), True
        yield query("b", False), False
        yield query("c", True), True
        yield query("d", False), True
        raise AssertionError("pool consumed after the set is full")

    assert pick_golden(results(), plain=2, coupled=1) == [
        {"file": "a.25.000fps.phash", "coupled": False},
        {"file": "c.25.000fps.phash", "coupled": True},
        {"file": "d.25.000fps.phash", "coupled": False},
    ]


def test_pick_golden_short_pool() -> None:
    assert pick_golden(iter([(query("a", True), True)]), plain=1, coupled=1) == [
        {"file": "a.25.000fps.phash", "coupled": True}
    ]


def run_cut_queries(base: Path, *args: str) -> list[dict]:
    out = subprocess.run(
        ["python3", "-I", "-", *args],
        input=o2switch_e2e.CUT_QUERIES,
        cwd=base,
        capture_output=True,
        text=True,
        check=True,
    )
    return [json.loads(line) for line in out.stdout.splitlines()]


@pytest.fixture
def remote_base(tmp_path: Path) -> Path:
    """o2switch layout: 3 packed .phash (one coupled in French, one in English), 1 not packed yet."""
    phash = tmp_path / "data" / "__PHASH__"
    phash.mkdir(parents=True)
    names = [
        "Fr.S01E01.25.000fps.phash",
        "En.S01E01.25.000fps.phash",
        "Plain.2010.25.000fps.phash",
        "New.25.000fps.phash",
    ]
    for i, name in enumerate(names):
        (phash / name).write_bytes(b"".join((i * 1000 + n).to_bytes(8, "big") for n in range(1000)))
        mtime = 3000 if name.startswith("New") else 1000
        os.utime(phash / name, (mtime, mtime))
    (tmp_path / "data" / "catalog.dbphash").write_bytes(b"")
    os.utime(tmp_path / "data" / "catalog.dbphash", (2000, 2000))
    (tmp_path / "data" / "SRT").mkdir()
    registry = {
        "couplings": [
            {"phash_filename": names[0], "subtitles": [{"lang": "fr"}]},
            {"phash_filename": names[1], "subtitles": [{"lang": "en"}]},
        ]
    }
    (tmp_path / "data" / "SRT" / "couplings.json").write_text(json.dumps(registry))
    return tmp_path


def test_cut_queries_pool_splits_packed_files_by_french_coupling(remote_base: Path) -> None:
    pool = run_cut_queries(remote_base, "pool", "4", "10", "10")
    assert sorted((q["_file"], q["_coupled"]) for q in pool) == [
        ("En.S01E01.25.000fps.phash", False),
        ("Fr.S01E01.25.000fps.phash", True),
        ("Plain.2010.25.000fps.phash", False),
    ]
    plain = [q["_file"] for q in pool if not q["_coupled"]]
    assert plain == sorted(plain, key=lambda f: hashlib.sha256(f.encode()).hexdigest())
    assert all(len(q["query_fingerprint"]) == 100 and q["fps"] == 25.0 and q["language"] == "fr" for q in pool)
    assert [q["_stem"] for q in pool if q["_coupled"]] == ["Fr.S01E01"]
    assert len(run_cut_queries(remote_base, "pool", "4", "1", "0")) == 1


def test_cut_queries_golden_replays_the_fixed_set(remote_base: Path) -> None:
    (remote_base / "etc").mkdir()
    golden = [
        {"file": "Fr.S01E01.25.000fps.phash", "coupled": True},
        {"file": "Gone.25.000fps.phash", "coupled": False},
    ]
    (remote_base / "etc" / "e2e_golden.json").write_text(json.dumps(golden))
    first, second = run_cut_queries(remote_base, "golden", "4")
    assert (first["_file"], first["_coupled"], first["_stem"]) == ("Fr.S01E01.25.000fps.phash", True, "Fr.S01E01")
    assert first["query_fingerprint"][0] == f"0x{450:016x}"
    assert second == {"_missing": True}
    assert run_cut_queries(remote_base, "golden", "4")[0] == first


class FakeGolden(Recette):
    """Serves a fixed set from `queries` and answers every recognize with `resp`; records SSH commands."""

    def __init__(self, golden_exists: bool, queries: list[dict], resp: dict) -> None:
        super().__init__("http://gateway", "key")
        self.golden_exists = golden_exists
        self.queries = queries
        self.resp = resp
        self.commands: list[str] = []
        self.written: str | None = None

    def ssh(self, cmd: str, stdin: str | None = None) -> subprocess.CompletedProcess[str]:
        self.commands.append(cmd)
        if cmd.startswith("test -s"):
            return subprocess.CompletedProcess([], 0 if self.golden_exists else 1, "", "")
        if " golden " in cmd or " pool " in cmd:
            return subprocess.CompletedProcess([], 0, "".join(json.dumps(q) + "\n" for q in self.queries), "")
        self.written = stdin
        return subprocess.CompletedProcess([], 0, "", "")

    def http(self, path: str, auth: bool = True, payload: dict | None = None) -> tuple[int, bytes]:
        assert payload is not None and not any(k.startswith("_") for k in payload)
        return 200, json.dumps({**self.resp, "timer_seconds": 0.1}).encode()


FULL = [query(f"Film.{i}", False) for i in range(o2switch_e2e.GOLDEN_PLAIN)] + [
    query("Film", True) for _ in range(o2switch_e2e.GOLDEN_COUPLED)
]
OWN_FILM = [{"source": "certified", "release": "Film.25.000fps.phash"}]


@pytest.mark.parametrize(
    ("queries", "resp", "failed"),
    [
        (FULL, answer("Film", None, None, OWN), 1),
        (FULL[:-1], answer("Film", None, None), 2),
        (FULL[:-1] + [{"_missing": True}], answer("Film", None, None), 2),
    ],
    ids=["coupled-foreign-subtitles", "set-too-short", "file-missing"],
)
def test_recognize_replays_existing_golden_set(queries: list[dict], resp: dict, failed: int) -> None:
    recette = FakeGolden(True, queries, resp)
    recette.recognize()
    assert recette.failed == failed
    assert not any(" pool " in c for c in recette.commands)


def test_recognize_all_pass() -> None:
    recette = FakeGolden(True, FULL, answer("Film", None, None, OWN_FILM))
    recette.recognize()
    assert recette.failed == 0


def test_recognize_builds_missing_golden_set_then_replays_it() -> None:
    recette = FakeGolden(False, FULL, answer("Film", None, None, OWN_FILM))
    recette.recognize()
    assert recette.failed == 0
    assert json.loads(recette.written or "") == [{"file": q["_file"], "coupled": q["_coupled"]} for q in FULL]
    assert any(" golden " in c for c in recette.commands)
