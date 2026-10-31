import itertools
import subprocess

import pytest

from scripts import o2switch_e2e
from scripts.o2switch_e2e import Recette, right_media


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
    ],
)
def test_right_media(media_match: str, stem: str, expected: bool) -> None:
    assert right_media(media_match, stem) is expected
