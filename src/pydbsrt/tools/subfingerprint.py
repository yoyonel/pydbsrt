"""
"""
import dataclasses
from dataclasses import dataclass
from typing import Iterator, Union

from imagehash import ImageHash
from pysrt.srtitem import SubRipTime

from pydbsrt.tools.subreader import SubReader

#
from pydbsrt.tools.videofingerprint import VideoFingerprint


def subriptime_to_frame(srt: SubRipTime) -> int:
    total_seconds = (
        srt.hours * 60 * 60 + srt.minutes * 60 + srt.seconds + srt.milliseconds / 1000.0
    )
    return int(total_seconds * 25)


@dataclass(init=True, repr=False, eq=False, order=False, unsafe_hash=False, frozen=True)
class SubFingerprint:
    index: int
    id_frame: int
    fp: ImageHash

    def __getitem__(self, item):
        return self.__getattribute__(
            dataclasses.fields(self)[item].name if isinstance(item, int) else item
        )


@dataclass(init=True, repr=False, eq=False, order=False, unsafe_hash=False, frozen=True)
class SubFingerprints:
    """"""

    sub_reader: SubReader
    imghash_reader: Union[VideoFingerprint, Iterator[ImageHash]]

    def __iter__(self) -> Iterator[SubFingerprint]:
        # we suppose subtitles generator in order
        fp = next(self.imghash_reader)
        id_frame = 0
        # iter on subtitles generator
        for subtitle in self.sub_reader:
            # get start, end timecodes
            tc_start, tc_end = subtitle.start, subtitle.end
            frame_start, frame_end = (
                subriptime_to_frame(tc_start),
                subriptime_to_frame(tc_end),
            )
            try:
                while not (id_frame <= frame_start <= id_frame + 1):
                    fp = next(self.imghash_reader)
                    id_frame += 1
                yield SubFingerprint(subtitle.index, id_frame, fp)
                while not (id_frame <= frame_end <= id_frame + 1):
                    fp = next(self.imghash_reader)
                    id_frame += 1
                    yield SubFingerprint(subtitle.index, id_frame, fp)
                yield SubFingerprint(subtitle.index, id_frame, fp)
            except StopIteration:
                break
