"""
https://stackoverflow.com/questions/8290397/how-to-split-an-iterable-in-constant-size-chunks
"""
from itertools import chain, islice
from typing import Any, Iterator


def chunks(iterable: Iterator[Any], size) -> Iterator[Any]:
    """

    Args:
        iterable:
        size:

    Returns:

    """
    iterator = iter(iterable)
    for first in iterator:
        yield list(chain([first], islice(iterator, size - 1)))
