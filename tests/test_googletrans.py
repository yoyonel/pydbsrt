import pytest
from googletrans import Translator


@pytest.mark.skip
@pytest.mark.parametrize(
    "text,dest,src,expected",
    [
        ("Hello", "French", "ENGLISH", "Salut"),
        ("Good Morning", "French", "ENGLISH", "Bonjour"),
    ],
)
def test_googletrans(text, dest, src, expected):
    translator = Translator()
    result = translator.translate(text, src=src, dest=dest)
    assert result.text == expected
