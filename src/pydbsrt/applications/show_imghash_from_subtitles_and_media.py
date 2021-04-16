# -*- coding: utf-8 -*-
"""
➜ poetry run python src/pydbsrt/app_cli.py show-imghash-from-subtitles-and-media --help
Usage: app_cli.py show-imghash-from-subtitles-and-media [OPTIONS]

Options:
  -s, --subtitles PATH  Path to subtitles file  [required]
  -m, --media PATH      Path to media  [required]
  -h, --help            Show this message and exit.

# Description
À partir d'un média (video) + un des sous-titres (synchronisés avec la vidéo)
le script produit dans la console des séries d'images hashes correspondant aux tunnels (temporels) de sous-titres

# Example
➜ poetry run python /src/pydbsrt/app_cli.py \
    show-imghash-from-subtitles-and-media \
        --subtitles data/big_buck_bunny_trailer_480p.en.srt \
        --media data/big_buck_bunny_trailer_480p.webm
The frame size for reading (32, 32) is different from the source frame size (854, 480).
* index subtitle: 1 - first frame: 0
d5d52ad52ad42ad4 d5dc88f72ac880f6 d5dd88f22acd80f2 d5dd98b32acc80b3
d5cd8cf22acd84b2 d5cc98f32acc84b3 d5cc8cb36acc84b3 d5cc8cb36acc84b3
d5c88cb37b4c84b3 d5c88cb37a4c85b3 d5c89cb36a4c94b3 d5889cb36b4c94b3
[...]
* index subtitle: 8 - first frame: 776
f59a93246a9b9564 f59a93246e9b8564 f58a93246a9b9565 f59a93246e9b8564
f59a93246e9b8564 f59a93246e9b8564 f59a93246e9b8564 f59a93246e9a8565
f59a93246e9a8565 f59a93246e9b8564 f59a93646a9a8565 f59a91646e9a8565
[...]
d5d3806d6ad381e5 d5d380fc6ad381e4 d5d281bd3ad281b5 d5d52ad52ad42ad4
d5d52ad52ad42ad4

"""

import click

# https://pypi.org/project/click-pathlib/
import click_pathlib
from imageio_ffmpeg import read_frames
from rich.console import Console

from pydbsrt.services.extended_subtitles import show_subtitles_fingerprints
from pydbsrt.tools.ffmpeg_tools.ffmeg_extract_frame import rawframe_to_imghash

console = Console()


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
