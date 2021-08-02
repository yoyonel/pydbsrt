"""
"""
import dataclasses
import itertools
import math
from dataclasses import dataclass
from typing import AsyncIterator, Callable, Iterator, Union

from imagehash import ImageHash
from pysrt.srtitem import SubRipTime

from pydbsrt.services.models import PHashMedia
from pydbsrt.tools.subreader import SubReader

#
from pydbsrt.tools.videofingerprint import VideoFingerprint


def subriptime_to_frame(srt: SubRipTime, fps: float = 25.0, cast_to_int: Callable[[float], int] = int) -> int:
    total_seconds = srt.hours * 60 * 60 + srt.minutes * 60 + srt.seconds + srt.milliseconds / 1000.0
    return cast_to_int(total_seconds * fps)


@dataclass(init=True, repr=False, eq=False, order=False, unsafe_hash=False, frozen=True)
class SubFingerprint:
    index: int
    offset_frame: int
    img_hash: ImageHash

    def __getitem__(self, item):
        return self.__getattribute__(dataclasses.fields(self)[item].name if isinstance(item, int) else item)


@dataclass(init=True, repr=False, eq=False, order=False, unsafe_hash=False, frozen=True)
class SubFingerprints:
    sub_reader: SubReader
    imghash_reader: Union[VideoFingerprint, Iterator[ImageHash], Iterator[int]]

    def __iter__(self) -> Iterator[SubFingerprint]:
        """
        Return fingerprint of frame recover by subtitles

        TODO
        Il faudrait faire une version plus stable/robuste avec une prise en compte globale des sous-titres,
        et ne pas considérer forcément que le SRT est bien construit
        => refaire le tri des sous-titres et gérer (manuellement) les unions des intervalles de timecodes
        IL faudrait mettre en place une résolution/arithmétique d'intervalles de timecodes (de sous-titres)
        Ça devrait être jouable avec une résolution algébrique (plus fine que la résolution numérique utilisée
        actuellement dans les tests unitaires).
        """
        it_imghash = self.imghash_reader

        # we suppose subtitles generator in order
        try:
            img_hash = next(it_imghash)
        except StopIteration:
            return
        it_imghash = itertools.chain([img_hash], it_imghash)

        # debug_frames_start_end = defaultdict(list)

        offset_frame = -1
        # iter on subtitles generator
        for subtitle in self.sub_reader:
            # get start, end timecodes
            tc_start, tc_end = subtitle.start, subtitle.end
            frame_start, frame_end = (
                subriptime_to_frame(tc_start, cast_to_int=math.floor),
                subriptime_to_frame(tc_end, cast_to_int=math.ceil),
            )
            try:
                if offset_frame < frame_start:
                    # go to the first subtitle frame offset
                    for offset_frame, img_hash in enumerate(it_imghash, start=offset_frame + 1):
                        if offset_frame >= frame_start:
                            break

                # yield the first frame+phash
                yield SubFingerprint(subtitle.index, offset_frame, img_hash)
                # debug_frames_start_end[subtitle.index].append(
                #     ((frame_start, frame_end), offset_frame)
                # )

                # (loop ...) iterate on images hashes frames
                for offset_frame, img_hash in enumerate(it_imghash, start=offset_frame + 1):
                    # yield the frame+phash
                    yield SubFingerprint(subtitle.index, offset_frame, img_hash)
                    # debug_frames_start_end[subtitle.index].append(
                    #     ((frame_start, frame_end), offset_frame)
                    # )
                    # (loop ...) until the last subtitle frame offset
                    if offset_frame >= frame_end:
                        break
            except StopIteration:
                break


async def anext(ait):
    return await ait.__anext__()


@dataclass(init=True, repr=False, eq=False, order=False, unsafe_hash=False, frozen=True)
class ASyncSubFingerprints:
    sub_reader: SubReader
    phash_media_reader: AsyncIterator[PHashMedia]

    async def stream(self) -> AsyncIterator[SubFingerprint]:
        ait_phash_media = self.phash_media_reader

        # async def get_first_and_reinsert(async_iterator: AsyncIterator) -> Tuple[Any, AsyncIterable]:
        #     try:
        #         first_elem = await anext(async_iterator)
        #     except StopIteration:
        #         return
        #
        #     async def _rebuild_async_iterator():
        #         yield first_elem
        #         async for elem in async_iterator:
        #             yield elem
        #
        #     return first_elem, _rebuild_async_iterator()
        #
        # img_hash, ait_imghash = await get_first_and_reinsert(ait_imghash)
        # img_hash = -1
        # offset_frame = -1

        phash_media = await anext(ait_phash_media)

        # iter on subtitles generator
        for subtitle in self.sub_reader:
            # get start, end timecodes
            tc_start, tc_end = subtitle.start, subtitle.end
            frame_start, frame_end = (
                subriptime_to_frame(tc_start, cast_to_int=math.floor),
                subriptime_to_frame(tc_end, cast_to_int=math.ceil),
            )
            try:
                while phash_media.frame_offset < frame_start:
                    try:
                        phash_media = await anext(ait_phash_media)
                    except StopAsyncIteration:
                        break
                # yield the first frame+phash
                yield SubFingerprint(subtitle.index, phash_media.frame_offset, phash_media.p_hash)
                # (loop ...) iterate on images hashes frames
                while phash_media.frame_offset < frame_end:
                    try:
                        phash_media = await anext(ait_phash_media)
                    except StopAsyncIteration:
                        break
                    yield SubFingerprint(subtitle.index, phash_media.frame_offset, phash_media.p_hash)
            except StopIteration:
                break
