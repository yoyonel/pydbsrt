import dataclasses
from dataclasses import dataclass


@dataclass(frozen=True)
class SubFrameRecord:
    index: int  # subtitles index
    start_frame_offset: int
    end_frame_offset: int
    subtitles_id: int

    text: str = dataclasses.field(default="")

    def __getitem__(self, item):
        return self.__getattribute__(dataclasses.fields(self)[item].name if isinstance(item, int) else item)


@dataclass(frozen=True)
class SubtitlesRecord:
    __table_name__ = "subtitles"

    subtitles_hash: str
    name: str

    # FOREIGN KEYS
    media_id: int

    # OPTIONALS
    lang: str = "FR"
