# -*- coding: utf-8 -*-
"""
"""
from pathlib import Path

import click

# https://pypi.org/project/click-pathlib/
import click_pathlib
from loguru import logger
from rich.console import Console

from pydbsrt.services.db_frames import agen_p_hash_from_media_in_db
from pydbsrt.services.extended_subtitles import async_show_subtitles_fingerprints
from pydbsrt.tools.aio_filehash import aio_hashfile
from pydbsrt.tools.coro import coroclick

console = Console()


async def do_show_imghash_from_subtitles_and_media_in_db(subtitles: Path, binary_img_hash_file: Path):
    media_hash = await aio_hashfile(binary_img_hash_file, hexdigest=True)
    agen_phash_media = agen_p_hash_from_media_in_db(media_hash)
    await async_show_subtitles_fingerprints(subtitles, agen_phash_media)


@click.command(short_help="")
@click.option(
    "--subtitles",
    "-s",
    required=True,
    type=click_pathlib.Path(exists=True, readable=True, resolve_path=True, allow_dash=False),
    help="Path to subtitles file",
)
@click.option(
    "--binary-img-hash-file",
    "-r",
    required=True,
    type=click_pathlib.Path(exists=True, readable=True, resolve_path=True, allow_dash=False),
    help="Path to binary image hashes media (just for id_hash)",
)
@coroclick
@logger.catch
async def show_imghash_from_subtitles_and_media_in_db(subtitles, binary_img_hash_file):
    await do_show_imghash_from_subtitles_and_media_in_db(subtitles, binary_img_hash_file)
