# -*- coding: utf-8 -*-
"""
"""

import click

# https://pypi.org/project/click-pathlib/
import click_pathlib
from loguru import logger
from rich.console import Console

import pydbsrt.services.extended_subtitles as srv_extended_srt
from pydbsrt.tools.coro import coroclick

console = Console()


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
    await srv_extended_srt.show_imghash_from_subtitles_and_media_in_db(subtitles, binary_img_hash_file)
