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

import click

# https://pypi.org/project/click-pathlib/
import click_pathlib
from loguru import logger
from rich.console import Console

import pydbsrt.services as services

console = Console()


@click.command(short_help="")
@click.option(
    "--media",
    "-r",
    required=True,
    # type=click_pathlib.Path(exists=True, readable=True, resolve_path=True, allow_dash=False),
    type=click_pathlib.Path(),
    help="Path to media",
)
@click.option("--output-file", "-o", default=None, help="File where to write images hashes.")
@logger.catch
def export_imghash_from_media(media, output_file):
    services.export_imghash_from_media(media, output_file)
