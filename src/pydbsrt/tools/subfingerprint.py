"""
"""
import dataclasses
import itertools
import math
from dataclasses import dataclass
from typing import Callable, Iterator, Union

from imagehash import ImageHash
from pydbsrt.tools.subreader import SubReader
#
from pydbsrt.tools.videofingerprint import VideoFingerprint
from pysrt.srtitem import SubRipTime


def subriptime_to_frame(
        srt: SubRipTime,
        fps: float = 25.0,
        cast_to_int: Callable[float, int] = int
) -> int:
    total_seconds = (
            srt.hours * 60 * 60 + srt.minutes * 60 + srt.seconds + srt.milliseconds / 1000.0
    )
    return cast_to_int(total_seconds * fps)


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
    sub_reader: SubReader
    imghash_reader: Union[VideoFingerprint, Iterator[ImageHash], Iterator[int]]

    def __iter__(self) -> Iterator[SubFingerprint]:
        it_imghash = self.imghash_reader

        # we suppose subtitles generator in order
        try:
            img_hash = next(it_imghash)
        except StopIteration:
            return
        it_imghash = itertools.chain([img_hash], it_imghash)

        offset_frame = 0
        # iter on subtitles generator
        for subtitle in self.sub_reader:
            # get start, end timecodes
            tc_start, tc_end = subtitle.start, subtitle.end
            frame_start, frame_end = (
                subriptime_to_frame(tc_start, cast_to_int=math.floor),
                subriptime_to_frame(tc_end, cast_to_int=math.ceil),
            )
            try:
                # go to the first subtitle frame offset
                for img_hash in it_imghash:
                    offset_frame += 1
                    if offset_frame >= frame_start:
                        break
                # yield the first frame+phash
                yield SubFingerprint(subtitle.index, offset_frame, img_hash)
                # (loop ...) iterate on images hashes frames
                for img_hash in it_imghash:
                    offset_frame += 1
                    # yield the frame+phash
                    yield SubFingerprint(subtitle.index, offset_frame, img_hash)
                    # (loop ...) until the last subtitle frame offset
                    if offset_frame >= frame_end:
                        break
            except StopIteration:
                break
