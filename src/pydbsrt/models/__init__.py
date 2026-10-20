from dataclasses import astuple
from typing import Any, cast


class UnpackIterable:
    # [Implement packing/unpacking in an object](https://stackoverflow.com/a/70753113)
    def __iter__(self):
        return iter(astuple(cast(Any, self)))
