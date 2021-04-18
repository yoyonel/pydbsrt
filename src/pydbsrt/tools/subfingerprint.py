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
    offset_frame: int
    img_hash: ImageHash

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
        img_hash = next(self.imghash_reader)
        offset_frame = 0
        # iter on subtitles generator
        for subtitle in self.sub_reader:
            # get start, end timecodes
            tc_start, tc_end = subtitle.start, subtitle.end
            frame_start, frame_end = (
                subriptime_to_frame(tc_start),
                subriptime_to_frame(tc_end),
            )
            try:
                while not (offset_frame <= frame_start <= offset_frame + 1):
                    img_hash = next(self.imghash_reader)
                    offset_frame += 1
                yield SubFingerprint(subtitle.index, offset_frame, img_hash)
                while not (offset_frame <= frame_end <= offset_frame + 1):
                    img_hash = next(self.imghash_reader)
                    offset_frame += 1
                    yield SubFingerprint(subtitle.index, offset_frame, img_hash)
                yield SubFingerprint(subtitle.index, offset_frame, img_hash)
            except StopIteration:
                break
