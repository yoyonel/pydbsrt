from typing import Callable

from rich.emoji import EMOJI
from fuzzywuzzy import fuzz


def find_emoji(
    emoji_desc: str, func_str_compare: Callable[[str, str], int] = fuzz.ratio
) -> str:
    """
    >>> find_emoji("warning")
    '⚠'
    >>> find_emoji("watermel")
    '🍉'
    >>> find_emoji("oki doki", func_str_compare=fuzz.partial_ratio)
    '🆗'
    """
    return sorted(EMOJI.items(), key=lambda kv: func_str_compare(kv[0], emoji_desc))[
        -1
    ][1]
