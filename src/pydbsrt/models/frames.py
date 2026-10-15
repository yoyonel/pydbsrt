from dataclasses import dataclass
from typing import Dict, Iterator

from pydbsrt.models import UnpackIterable


@dataclass(frozen=True)
class BuildReaderFrameResult(UnpackIterable):
    reader: Iterator[bytes]
    meta: Dict


@dataclass(frozen=True)
class BinaryImageHash(UnpackIterable):
    img_hash: int
    offset_frame: int
    media_id: int
