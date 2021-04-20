# -*- coding: utf-8 -*-
"""
➜ poetry python src/pydbsrt/app_cli.py export-imghash-from-media --help
Usage: app_cli.py export-imghash-from-media [OPTIONS]

Options:
  -r, --media PATH        Path to media  [required]
  -o, --output-file TEXT  File where to write images hashes.
  -h, --help              Show this message and exit.

# Description
Export hashes (perceptive) images compute from an input video to output (binary) file.

# Example
➜ poetry python src/pydbsrt/app_cli.py \
    export-imghash-from-media \
        --media data/big_buck_bunny_trailer_480p.webm
The frame size for reading (32, 32) is different from the source frame size (854, 480).
{
    'ffmpeg_version': '4.2.2-static https://johnvansickle.com/ffmpeg/ built with gcc 8 (Debian 8.3.0-6)',
    'codec': 'vp8,',
    'pix_fmt': 'yuv420p(progressive)',
    'fps': 25.0,
    'source_size': (854, 480),
    'size': (32, 32),
    'duration': 33.0
}
Chunk size (nb frames): 375
output_file: /tmp/big_buck_bunny_trailer_480p.phash
big_buck_bunny_trailer_480p.webm ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━ 100.0% • 1,125/825 bytes • 4.1 kB/s • 0:00:00/0:00:00
"""
from itertools import islice
from pathlib import Path

import click

# https://pypi.org/project/click-pathlib/
import click_pathlib
from numpy.ma import floor
from rich.console import Console
from rich.progress import (
    BarColumn,
    DownloadColumn,
    Progress,
    TextColumn,
    TransferSpeedColumn,
)

from pydbsrt.services.reader_frames import build_reader_frames
from pydbsrt.tools.chunk import chunks
from pydbsrt.tools.ffmpeg_tools.ffmeg_extract_frame import rawframe_to_imghash
from pydbsrt.tools.imghash import imghash_to_bytes
from pydbsrt.tools.rich_colums import TimeElapsedOverRemainingColumn

console = Console()


@click.command(short_help="")
@click.option(
    "--media",
    "-r",
    required=True,
    type=click_pathlib.Path(
        exists=True, readable=True, resolve_path=True, allow_dash=False
    ),
    help="Path to media",
)
@click.option(
    "--output-file", "-o", default=None, help="File where to write images hashes."
)
def export_imghash_from_media(media, output_file):
    ################################################################
    # PROCESSING                                                   #
    ################################################################
    # Read a video file
    reader, meta = build_reader_frames(media)
    console.print(meta)
    nb_frames_to_read = int(floor(meta["fps"] * meta["duration"]))
    gen_frame_hash = map(rawframe_to_imghash, reader)
    gen_frame_hash = islice(
        gen_frame_hash, nb_frames_to_read + 0 * int(meta["fps"] * 60)
    )
    chunk_nb_seconds = 15
    chunk_size = int(meta["fps"] * chunk_nb_seconds)
    console.print(f"Chunk size (nb frames): {chunk_size}")
    ################################################################
    # EXPORT                                                       #
    ################################################################
    output_file = (
        Path(output_file)
        if output_file
        else Path("/tmp/") / media.with_suffix(".phash").name
    )
    output_file.unlink(missing_ok=True)
    console.print(f"output_file: {str(output_file)}")
    progress = Progress(
        TextColumn("[bold blue]{task.fields[filename]}", justify="right"),
        BarColumn(bar_width=None),
        "[progress.percentage]{task.percentage:>3.1f}%",
        "•",
        DownloadColumn(),
        "•",
        TransferSpeedColumn(),
        "•",
        TimeElapsedOverRemainingColumn(),
        console=console,
    )
    task_id = progress.add_task(
        "build&export images hashes", filename=media.name, start=True
    )
    progress.update(task_id, total=nb_frames_to_read)
    with progress:
        for chunk_frames_hashes in chunks(gen_frame_hash, chunk_size):
            with output_file.open("ab") as fo:
                for frame_hash_binary in map(imghash_to_bytes, chunk_frames_hashes):
                    fo.write(frame_hash_binary)
            progress.update(task_id, advance=chunk_size, refresh=True)
