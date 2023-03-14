from dataclasses import dataclass
from typing import List

from contexttimer import Timer


@dataclass(frozen=True)
class MatchedFrame:
    frame_offset: int
    media_id: int


@dataclass(frozen=True)
class ResultSearchRecord:
    search_phash: int
    search_offset: int
    matches: List[MatchedFrame]


@dataclass(frozen=True)
class ResultSearch:
    records: List[ResultSearchRecord]
    timer: Timer
