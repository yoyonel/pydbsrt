# -*- coding: utf-8 -*-
"""
➜ poetry run python src/pydbsrt/app_cli.py export-imghash-from-subtitles-and-media --help
Usage: app_cli.py export-imghash-from-subtitles-and-media [OPTIONS]

Options:
  -s, --subtitles PATH    Path to subtitles file  [required]
  -m, --media PATH        Path to media  [required]
  -o, --output-file TEXT  File where to write images hashes.
  -h, --help              Show this message and exit.

# Description
À partir d'un média (video) + un des sous-titres (synchronisés avec la vidéo)
le script exporte  dans un fichier output les images hashes correspondant aux tunnels (temporels) de sous-titres

# Example
➜ poetry run python src/pydbsrt/app_cli.py \
    export-imghash-from-subtitles-and-media \
        --subtitles data/big_buck_bunny_trailer_480p.en.srt \
        --media data/big_buck_bunny_trailer_480p.webm
The frame size for reading (32, 32) is different from the source frame size (854, 480).

✅  Export binary images hashes to: /tmp/big_buck_bunny_trailer_480p.en.phash

➜ hexdump /tmp/big_buck_bunny_trailer_480p.en.phash | head
0000000 d5d5 d52a d42a d42a dcd5 f788 c82a f680
0000010 ddd5 f288 cd2a f280 ddd5 b398 cc2a b380
0000020 cdd5 f28c cd2a b284 ccd5 f398 cc2a b384
0000030 ccd5 b38c cc6a b384 ccd5 b38c cc6a b384
0000040 c8d5 b38c 4c7b b384 c8d5 b38c 4c7a b385
0000050 c8d5 b39c 4c6a b394 88d5 b39c 4c6b b394
0000060 88d5 b39c 4c6b b394 ccd5 b38c 4c6b b384
0000070 8cd5 b38c 4c6b b394 0cd5 b39c 4c6a b395
0000080 4cd5 b38c 4c6a b395 0cd5 b39c 4c6a b395
0000090 0cd5 b39c 4c6a b395 0cd5 b39c 4c6a b395

"""

import click

# https://pypi.org/project/click-pathlib/
import click_pathlib

from pydbsrt.services.extended_subtitles import export_extended_subtitles


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
    export_extended_subtitles(subtitles, media, output_file)
