from itertools import groupby, chain
from operator import itemgetter
from pathlib import Path

from more_itertools import grouper
from rich.console import Console
from yaspin import yaspin
from yaspin.spinners import Spinners

from pydbsrt.services.reader_frames import build_reader_frames
from pydbsrt.tools.chunk import chunks
from pydbsrt.tools.ffmpeg_tools.ffmeg_extract_frame import rawframe_to_imghash
from pydbsrt.tools.imghash import imghash_to_binary
from pydbsrt.tools.subfingerprint import SubFingerprints
from pydbsrt.tools.subreader import SubReader

console = Console()


def export_extended_subtitles(
    srt_path: Path,
    media: Path,
    output_file: Path,
    chunk_size: int = 25,
) -> None:
    #
    reader, _ = build_reader_frames(media)
    it_sub_fingerprints = SubFingerprints(
        sub_reader=SubReader(srt_path), imghash_reader=map(rawframe_to_imghash, reader)
    )
    #
    it_binary_imghash = (
        imghash_to_binary(sub_fingerprint.img_hash)
        for sub_fingerprint in it_sub_fingerprints
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
    srt_path: Path, media: Path, nb_fingerprints_by_row: int = 4
) -> None:
    reader, _ = build_reader_frames(media)
    it_sub_fingerprints = SubFingerprints(
        sub_reader=SubReader(srt_path), imghash_reader=map(rawframe_to_imghash, reader)
    )
    gb_sub_fingerprints = groupby(it_sub_fingerprints, key=itemgetter("index"))
    for index_subtitle, it_indexed_sub_fingerprints in gb_sub_fingerprints:
        _, id_frame, fingerprint = next(it_indexed_sub_fingerprints)
        it_indexed_sub_fingerprints = chain(
            ((index_subtitle, id, fingerprint),), it_indexed_sub_fingerprints
        )
        console.print(f"* index subtitle: {index_subtitle} - first frame: {id_frame}")
        for chunk_fingerprints in grouper(
            (fingerprint for _, __, fingerprint in it_indexed_sub_fingerprints),
            nb_fingerprints_by_row,
        ):
            console.print(" ".join(map(str, filter(None, chunk_fingerprints))))
