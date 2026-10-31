import pytest

from scripts.o2switch_e2e import right_media


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
