# -*- coding: utf-8 -*-
"""
➜ poetry run python src/pydbsrt/app_cli.py import-subtitles-into-db \
    --subtitles [...]/video/Stalker.1979.1080p.BluRay.x264-USURY[rarbg]/stalker.1979.1080p.bluray.x264-usury.fr.srt \
    --binary-img-hash-file /home/latty/NAS/video/__PHASH__/stalker.1979.1080p.bluray.x264-usury.mkv.phash
✅  Insert subtitles=stalker.1979.1080p.bluray.x264-usury.fr into DB (0:00:01.63)

# Description

"""
import asyncio
from pathlib import Path

import click

# https://pypi.org/project/click-pathlib/
import click_pathlib
from rich.console import Console
from yaspin import yaspin
from yaspin.spinners import Spinners

import pydbsrt.services.database as database

console = Console()


async def run(subtitles: Path, binary_img_hash_file: Path):
    spinner = yaspin(
        Spinners.dots2, f"Insert subtitles={subtitles.stem} into DB", timer=True
    )
    nb_rows_inserted = await database.import_subtitles_into_db(
        subtitles,
        binary_img_hash_file,
        spinner,
        drop_before_inserting=True,
        check_before_inserting=False,
    )
    console.print(f"nb_rows_inserted = {nb_rows_inserted}")


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
    help="Path to binary image hash file (generated from media)",
)
def import_subtitles_into_db(subtitles, binary_img_hash_file):
    loop = asyncio.get_event_loop()
    loop.run_until_complete(run(subtitles, binary_img_hash_file))
