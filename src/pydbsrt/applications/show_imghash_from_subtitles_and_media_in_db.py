# -*- coding: utf-8 -*-
"""
"""
import asyncio
from typing import Iterator

import click

# https://pypi.org/project/click-pathlib/
import click_pathlib
from aiostream import stream
from imohash import hashfile
from rich.console import Console

from pydbsrt.services.db_frames import agen_p_hash_from_media_in_db
from pydbsrt.services.extended_subtitles import show_subtitles_fingerprints
from pydbsrt.tools.imghash import imghash_to_signed_int64

console = Console()


async def run(subtitles, binary_img_hash_file):
    media_hash = hashfile(binary_img_hash_file, hexdigest=True)
    async for p_hash in agen_p_hash_from_media_in_db(media_hash, limit=32):
        console.print(p_hash)


async def run2(subtitles, binary_img_hash_file):
    # TODO: Find a way to use an AsyncIterable as an Iterable
    media_hash = hashfile(binary_img_hash_file, hexdigest=True)
    agen_phash_media = agen_p_hash_from_media_in_db(media_hash, limit=32)
    it_p_hash: Iterator[int] = stream.map(agen_phash_media, (lambda phash_media: phash_media.p_hash))
    show_subtitles_fingerprints(
        subtitles,
        it_p_hash,
        fn_imghash_to=lambda fp: str(imghash_to_signed_int64(fp)),
    )


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
    help="Path to media",
)
def show_imghash_from_subtitles_and_media_in_db(subtitles, binary_img_hash_file):
    loop = asyncio.get_event_loop()
    loop.run_until_complete(run(subtitles, binary_img_hash_file))
