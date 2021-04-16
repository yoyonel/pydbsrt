from itertools import groupby, chain
from operator import itemgetter
from pathlib import Path
from typing import Iterator

# https://pypi.org/project/click-pathlib/
from imagehash import ImageHash
from more_itertools import grouper
from rich.console import Console
from yaspin import yaspin
from yaspin.spinners import Spinners

from pydbsrt.tools.chunk import chunks
from pydbsrt.tools.imghash import imghash_to_binary
from pydbsrt.tools.subfingerprint import SubFingerprints
from pydbsrt.tools.subreader import SubReader

console = Console()


def export_extended_subtitles(
    srt_path: Path,
    it_imghash: Iterator[ImageHash],
    output_file: Path,
    chunk_size: int = 25,
) -> None:
    it_sub_fingerprints = SubFingerprints(
        sub_reader=SubReader(srt_path), imghash_reader=it_imghash
    )
    it_binary_imghash = (
        imghash_to_binary(sub_fingerprint.fp) for sub_fingerprint in it_sub_fingerprints
    )
    ################################################################
    # EXPORT                                                       #
    ################################################################
    output_file = (
        Path(output_file)
        if output_file
        else Path("/tmp/") / srt_path.with_suffix(".phash").name
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
    srt_path: Path, it_imghash: Iterator[ImageHash], chunk_size: int = 25
) -> None:
    it_sub_fingerprints = SubFingerprints(
        sub_reader=SubReader(srt_path), imghash_reader=it_imghash
    )
    gb_sub_fingerprints = groupby(it_sub_fingerprints, key=itemgetter("index"))
    nb_fingerprints_by_chunk = 4
    for index_subtitle, it_indexed_sub_fingerprints in gb_sub_fingerprints:
        _, id_frame, fingerprint = next(it_indexed_sub_fingerprints)
        it_indexed_sub_fingerprints = chain(
            ((index_subtitle, id, fingerprint),), it_indexed_sub_fingerprints
        )
        console.print(f"* index subtitle: {index_subtitle} - first frame: {id_frame}")
        for chunk_fingerprints in grouper(
            (fingerprint for _, __, fingerprint in it_indexed_sub_fingerprints),
            nb_fingerprints_by_chunk,
        ):
            console.print(" ".join(map(str, filter(None, chunk_fingerprints))))