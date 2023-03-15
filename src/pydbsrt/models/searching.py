from dataclasses import dataclass
from typing import Set

from pydbsrt.models.phash import PHashMedia


@dataclass(frozen=True)
class PairedMatchedFrame:
    record_search_offset: int
    match_frame_offset: int


@dataclass(frozen=True)
class BuildSearchResult:
    media_found: bool
    media_name_search: str
    media_name_match: str
    media_id_match: int
    nb_offsets_match: int
    search_offsets_match: Set[int]
    match_frames_offsets: Set[int]
    timer_in_seconds: float
    confidence: float


@dataclass(frozen=True)
class SubFrameSearchResult(PHashMedia):
    frame_id: int
