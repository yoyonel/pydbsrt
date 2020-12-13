from pathlib import Path
from typing import Iterator

import click

# https://pypi.org/project/click-pathlib/
import click_pathlib
from imagehash import ImageHash
from imageio_ffmpeg import read_frames
from rich.console import Console
from yaspin import yaspin
from yaspin.spinners import Spinners

from pydbsrt.tools.chunk import chunks
from pydbsrt.tools.ffmpeg_tools.ffmeg_extract_frame import rawframe_to_imghash
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


@click.command(short_help="")
@click.option(
    "--subtitles",
    "-s",
    required=True,
    type=click_pathlib.Path(
        exists=True, readable=True, resolve_path=True, allow_dash=False
    ),
    help="Path to subtitles file",
)
@click.option(
    "--media",
    "-m",
    required=True,
    type=click_pathlib.Path(
        exists=True, readable=True, resolve_path=True, allow_dash=False
    ),
    help="Path to media",
)
@click.option(
    "--output-file", "-o", default=None, help="File where to write images hashes."
)
def export_imghash_from_subtitles_and_media(subtitles, media, output_file):
    # Read a video file
    reader = read_frames(
        media,
        input_params="-hide_banner -nostats -nostdin".split(" "),
        output_params=["-vf", "scale=width=32:height=32", "-pix_fmt", "gray"],
        bits_per_pixel=8,
    )
    reader.__next__()
    gen_frame_hash = map(rawframe_to_imghash, reader)
    export_extended_subtitles(
        srt_path=subtitles, it_imghash=gen_frame_hash, output_file=output_file
    )
