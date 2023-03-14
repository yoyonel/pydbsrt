from dataclasses import dataclass

from pydbsrt.models import UnpackIterable


@dataclass(frozen=True)
class ImportBinaryImageHashResult(UnpackIterable):
    media_id: int
    nb_frames_inserted: int


@dataclass(frozen=True)
class ImportSubtitlesResult(UnpackIterable):
    nb_row_inserted: int
