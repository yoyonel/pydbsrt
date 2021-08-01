# -*- coding: utf-8 -*-
"""
➜ poetry run python src/pydbsrt/app_cli.py import-images-hashes-into-db --help
Usage: app_cli.py import-images-hashes-into-db [OPTIONS]

Options:
  -r, --binary_img_hash_file PATH
                                  Path to media  [required]
  -h, --help                      Show this message and exit.

# Description
Import l'ensemble des images hashes (des frames d'un média) d'un fichier .phash (output du script `export_imghash_from_media`)
dans BKTreeDB la base de donnée PostgreSQL (avec index) spécialisé·e pour le stockage (et recherche) de perceptives hashes.

# Example
➜ /usr/bin/ls -1 /tmp/*.phash | \
    xargs -I {} poetry run python app_cli.py import-images-hashes-into-db -r "{}"

"""
import asyncio
from pathlib import Path

import click
import click_pathlib
from rich.console import Console
from rich.progress import BarColumn, DownloadColumn, Progress, TextColumn, TransferSpeedColumn

from pydbsrt.services.db_frames import import_binary_img_hash_to_db_async
from pydbsrt.tools.rich_colums import TimeElapsedOverRemainingColumn

console = Console()

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


async def run(binary_img_hash_file: Path) -> None:
    media_id, nb_frames_inserted = await import_binary_img_hash_to_db_async(binary_img_hash_file, progress)
    if nb_frames_inserted:
        console.print(f"count(frames where frames.media_id = {media_id})={nb_frames_inserted}")
    else:
        console.print(f"binary_img_hash_file={str(binary_img_hash_file)} already in DB (medias.id={media_id})")


@click.command(short_help="")
@click.option(
    "--binary_img_hash_file",
    "-r",
    required=True,
    type=click_pathlib.Path(exists=True, readable=True, resolve_path=True, allow_dash=False),
    help="Path to media",
)
def import_images_hashes_into_db(binary_img_hash_file):
    loop = asyncio.get_event_loop()
    loop.run_until_complete(run(binary_img_hash_file))
