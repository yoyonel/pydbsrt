# -*- coding: utf-8 -*-
"""
"""
import asyncio

import click

# https://pypi.org/project/click-pathlib/
import click_pathlib
from imohash import hashfile
from rich.console import Console

from pydbsrt.services.database import agen_p_hash_from_media_in_db

console = Console()


async def run(subtitles, binary_img_hash_file):
    media_hash = hashfile(binary_img_hash_file, hexdigest=True)
    async for p_hash in agen_p_hash_from_media_in_db(media_hash, limit=32):
        console.print(p_hash)


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
    "--binary-img-hash-file",
    "-r",
    required=True,
    type=click_pathlib.Path(
        exists=True, readable=True, resolve_path=True, allow_dash=False
    ),
    help="Path to media",
)
def show_imghash_from_subtitles_and_media_in_db(subtitles, binary_img_hash_file):
    loop = asyncio.get_event_loop()
    loop.run_until_complete(run(subtitles, binary_img_hash_file))
