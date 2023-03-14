from dataclasses import dataclass
from typing import Union

Hash = Union[str, bytes]


@dataclass(frozen=True)
class PHashMedia:
    p_hash: int
    frame_offset: int
