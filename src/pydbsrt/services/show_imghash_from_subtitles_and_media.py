from itertools import groupby, chain
from operator import itemgetter
from pathlib import Path
from typing import Iterator

import click

# https://pypi.org/project/click-pathlib/
import click_pathlib
from imagehash import ImageHash
from imageio_ffmpeg import read_frames
from more_itertools import grouper
from rich.console import Console

from pydbsrt.tools.ffmpeg_tools.ffmeg_extract_frame import rawframe_to_imghash
from pydbsrt.tools.subfingerprint import SubFingerprints
from pydbsrt.tools.subreader import SubReader

console = Console()


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
def show_imghash_from_subtitles_and_media(subtitles, media):
    # Read a video file
    reader = read_frames(
        media,
        input_params="-hide_banner -nostats -nostdin".split(" "),
        output_params=["-vf", "scale=width=32:height=32", "-pix_fmt", "gray"],
        bits_per_pixel=8,
    )
    reader.__next__()
    gen_frame_hash = map(rawframe_to_imghash, reader)
    show_subtitles_fingerprints(subtitles, gen_frame_hash)
