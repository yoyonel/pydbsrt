from itertools import groupby, chain
from operator import itemgetter
from pathlib import Path
from struct import unpack
from typing import Iterator, Callable

from imagehash import ImageHash
from more_itertools import grouper
from rich.console import Console
from yaspin import yaspin
from yaspin.spinners import Spinners

from pydbsrt.services.reader_frames import build_reader_frames
from pydbsrt.tools.chunk import chunks
from pydbsrt.tools.ffmpeg_tools.ffmeg_extract_frame import rawframe_to_imghash
from pydbsrt.tools.imghash import imghash_to_bytes
from pydbsrt.tools.subfingerprint import SubFingerprints
from pydbsrt.tools.subreader import SubReader

console = Console()

SIZE_IMG_HASH = 8


def export_extended_subtitles(
    subtitles: Path,
    media: Path,
    output_file: Path,
    chunk_size: int = 25,
) -> None:
    #
    reader, _ = build_reader_frames(media)
    it_sub_fingerprints = SubFingerprints(
        sub_reader=SubReader(subtitles), imghash_reader=map(rawframe_to_imghash, reader)
    )
    #
    it_binary_imghash = (
        imghash_to_bytes(sub_fingerprint.img_hash)
        for sub_fingerprint in it_sub_fingerprints
    )
    ################################################################
    # EXPORT                                                       #
    ################################################################
    output_file = (
        Path(output_file)
        if output_file
        else Path("/tmp/") / subtitles.with_suffix(".phash").name
    )
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


def show_subtitles_fingerprints(
    srt_path: Path,
    it_img_hash: Iterator[ImageHash],
    nb_fingerprints_by_row: int = 4,
    fn_imghash_to: Callable[[ImageHash], str] = str,
) -> None:
    it_sub_fingerprints = SubFingerprints(
        sub_reader=SubReader(srt_path), imghash_reader=it_img_hash
    )
    gb_sub_fingerprints = groupby(it_sub_fingerprints, key=itemgetter("index"))
    for index_subtitle, it_indexed_sub_fingerprints in gb_sub_fingerprints:
        _, id_frame, fingerprint = next(it_indexed_sub_fingerprints)
        it_indexed_sub_fingerprints = chain(
            ((index_subtitle, id_frame, fingerprint),), it_indexed_sub_fingerprints
        )
        console.print(f"* index subtitle: {index_subtitle} - first frame: {id_frame}")
        for chunk_fingerprints in grouper(
            (fingerprint for _, __, fingerprint in it_indexed_sub_fingerprints),
            nb_fingerprints_by_row,
        ):
            console.print(
                " ".join(fn_imghash_to(fp) for fp in filter(None, chunk_fingerprints))
            )


def read_extended_subtitles(
    binary_extended_subtitles: Path,
    chunk_size: int = 1024,
) -> Iterator[int]:
    with binary_extended_subtitles.open("rb") as fin:
        while chunk_img_hash := fin.read(SIZE_IMG_HASH * chunk_size):
            # https://docs.python.org/3/library/struct.html#byte-order-size-and-alignment
            for signed_int64_img_hash in unpack(
                f">{'q' * (len(chunk_img_hash) // SIZE_IMG_HASH)}", chunk_img_hash
            ):
                yield signed_int64_img_hash
