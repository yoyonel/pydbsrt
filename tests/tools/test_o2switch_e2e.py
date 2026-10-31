import itertools
import subprocess

import pytest

from scripts import o2switch_e2e
from scripts.o2switch_e2e import Recette, parse_releases, right_media


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
