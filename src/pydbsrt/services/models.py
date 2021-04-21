import dataclasses
from dataclasses import dataclass


@dataclass(frozen=True)
class PHashMedia:
    p_hash: int
    frame_offset: int


@dataclass(frozen=True)
class SubFrameRecord:
    index_subtitle: int
    start_frame_offset: int
    end_frame_offset: int
    subtitles_id: int

    text: str = dataclasses.field(default=str)

    def __getitem__(self, item):
        return self.__getattribute__(
            dataclasses.fields(self)[item].name if isinstance(item, int) else item
        )
