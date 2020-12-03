import logging
# from itertools import islice

import click
# https://pypi.org/project/click-pathlib/
import click_pathlib
from imageio_ffmpeg import read_frames
from numpy.ma import floor

from rich.console import Console
from rich.progress import track

from pydbsrt.tools.ffmpeg_tools.ffmeg_extract_frame import rawframe_to_imghash

console = Console()


@click.command(short_help="")
@click.option(
    "--media", "-r",
    required=True,
    type=click_pathlib.Path(exists=True, readable=True, resolve_path=True, allow_dash=False),
    help="Path to media"
)
@click.option("--output-file", "-o", default="img_hashes", help="File where to write images hashes.")
def export_imghash_from_media(media, output_file):
    console.log("")
    # Read a video file
    reader = read_frames(
        media,
        output_params=[
            "-vf", "scale=width=32:height=32",
            "-pix_fmt", "gray",
        ],
        bits_per_pixel=8,
    )
    meta = reader.__next__()  # meta data, e.g. meta["size"] -> (width, height)
    console.log(meta)
    nb_frames_to_read = floor(meta["fps"] * meta["duration"])
    for frame_hash in map(
            rawframe_to_imghash,
            track(reader, description=f"Build pHash from media={media.name}", total=nb_frames_to_read, console=console)
    ):
        console.log(frame_hash)


@click.group(context_settings={"help_option_names": ["-h", "--help"]})
@click.option("--log-level", default="WARN", help="set logging level")
def entry_point(log_level):
    logging.getLogger(__name__).setLevel(getattr(logging, log_level.upper()))


entry_point.add_command(export_imghash_from_media)

if __name__ == "__main__":
    entry_point()
