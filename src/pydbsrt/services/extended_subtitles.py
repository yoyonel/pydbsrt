from itertools import chain, groupby
from operator import itemgetter
from pathlib import Path
from struct import unpack
from tempfile import gettempdir
from typing import AsyncIterator, Callable, Iterator, Optional, Tuple, Union

import asyncstdlib
from imagehash import ImageHash
from more_itertools import grouper
from rich.console import Console
from yaspin import yaspin
from yaspin.spinners import Spinners

from pydbsrt.services.models import PHashMedia
from pydbsrt.services.reader_frames import build_reader_frames
from pydbsrt.tools.chunk import chunks
from pydbsrt.tools.ffmpeg_tools.ffmeg_extract_frame import rawframe_to_imghash
from pydbsrt.tools.imghash import imghash_to_bytes
from pydbsrt.tools.subfingerprint import ASyncSubFingerprints, SubFingerprint, SubFingerprints, anext
from pydbsrt.tools.subreader import SubReader

console = Console()

SIZE_IMG_HASH = 8


def export_extended_subtitles(
    subtitles: Path,
    media: Path,
    output_file: Optional[Path],
    chunk_size: int = 25,
) -> Path:
    #
    reader, _ = build_reader_frames(media)
    it_sub_fingerprints = SubFingerprints(
        sub_reader=SubReader(subtitles), imghash_reader=map(rawframe_to_imghash, reader)
    )
    #
    it_binary_imghash = (imghash_to_bytes(sub_fingerprint.img_hash) for sub_fingerprint in it_sub_fingerprints)
    ################################################################
    # EXPORT                                                       #
    ################################################################
    output_file = Path(output_file) if output_file else Path(gettempdir()) / subtitles.with_suffix(".phash").name
    output_file.unlink(missing_ok=True)
    console.print()
    # https://github.com/pavdmyt/yaspin#writing-messages
    with yaspin(
        Spinners.bouncingBall,
        text=f"Export binary images hashes to: {str(output_file)}",
    ) as spinner:
        for chunk_binary_imghash in chunks(it_binary_imghash, chunk_size):
            with output_file.open("ab") as fo:
                fo.write(b"".join(chunk_binary_imghash))
    spinner.ok("✅ ")

    return output_file


def show_subtitles_fingerprints(
    srt_path: Path,
    it_img_hash: Union[Iterator[ImageHash], Iterator[int]],
    nb_fingerprints_by_row: int = 4,
    fn_imghash_to: Callable[[ImageHash], str] = str,
) -> None:
    it_sub_fingerprints = SubFingerprints(sub_reader=SubReader(srt_path), imghash_reader=it_img_hash)
    gb_sub_fingerprints: Iterator[Tuple[int, Iterator[SubFingerprint]]] = groupby(
        it_sub_fingerprints, key=itemgetter("index")
    )
    for index_subtitle, it_indexed_sub_fingerprints in gb_sub_fingerprints:
        sub_fingerprint = next(it_indexed_sub_fingerprints)
        it_indexed_sub_fingerprints = chain(
            (SubFingerprint(index_subtitle, sub_fingerprint.offset_frame, sub_fingerprint.img_hash),),
            it_indexed_sub_fingerprints,
        )
        console.print(f"* index subtitle: {index_subtitle} - first frame: {sub_fingerprint.offset_frame}")
        it_fingerprint: Iterator[ImageHash] = (
            sub_fingerprint.img_hash for sub_fingerprint in it_indexed_sub_fingerprints
        )
        for chunk_fingerprints in grouper(it_fingerprint, nb_fingerprints_by_row):
            it_chunk_fingerprints: Iterator[ImageHash] = filter(None, chunk_fingerprints)
            console.print(" ".join(fn_imghash_to(img_hash) for img_hash in it_chunk_fingerprints))


async def async_show_subtitles_fingerprints(
    srt_path: Path,
    agen_phash_media: AsyncIterator[PHashMedia],
    nb_fingerprints_by_row: int = 4,
    fn_imghash_to: Callable[[ImageHash], str] = str,
) -> None:
    agen_sub_fingerprint = ASyncSubFingerprints(sub_reader=SubReader(srt_path), phash_media_reader=agen_phash_media)
    aio_gb_sub_fingerprints = asyncstdlib.groupby(agen_sub_fingerprint.stream(), key=itemgetter("index"))
    async for index_subtitle, ait_indexed_sub_fingerprints in aio_gb_sub_fingerprints:
        sub_fingerprint = await anext(ait_indexed_sub_fingerprints)

        console.print(f"* index subtitle: {index_subtitle} - first frame: {sub_fingerprint.offset_frame}")

        it_fingerprint = iter([sub_fingerprint.img_hash async for sub_fingerprint in ait_indexed_sub_fingerprints])

        for chunk_fingerprints in grouper(it_fingerprint, nb_fingerprints_by_row):
            it_chunk_fingerprints: Iterator[ImageHash] = filter(None, chunk_fingerprints)
            console.print(" ".join(fn_imghash_to(img_hash) for img_hash in it_chunk_fingerprints))


def read_extended_subtitles(
    binary_extended_subtitles: Path,
    chunk_size: int = 1024,
) -> Iterator[int]:
    with binary_extended_subtitles.open("rb") as fin:
        while chunk_img_hash := fin.read(SIZE_IMG_HASH * chunk_size):
            # https://docs.python.org/3/library/struct.html#byte-order-size-and-alignment
            for signed_int64_img_hash in unpack(f">{'q' * (len(chunk_img_hash) // SIZE_IMG_HASH)}", chunk_img_hash):
                yield signed_int64_img_hash
