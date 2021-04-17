# -*- coding: utf-8 -*-
"""
➜ poetry run python src/pydbsrt/app_cli.py search-imghash-in-db --help
Usage: app_cli.py search-imghash-in-db [OPTIONS]

Options:
  -p, --phash_file PATH   PHash stream to search in db  [required]
  -d, --distance INTEGER  Search distance to use
  -h, --help              Show this message and exit.

# Description

# Example

➜ poetry run python src/pydbsrt/search-imghash-in-db --phash_file /tmp/searching_phash.txt

"""
import asyncio
import sys
from pathlib import Path

import click
import click_pathlib
from rich.console import Console

from pydbsrt.services.matching import ResultSearch, search_phash_stream


console = Console()


@click.command(short_help="")
@click.option(
    "--phash_file",
    "-p",
    required=True,
    type=click_pathlib.Path(
        exists=True, readable=True, resolve_path=True, allow_dash=True
    ),
    help="PHash stream to search in db",
)
@click.option("--distance", "-d", default=1, type=int, help="Search distance to use")
def search_imghash_in_db(phash_file, distance):
    phash_stream = sys.stdin if phash_file == Path("-") else phash_file.open("r")
    loop = asyncio.get_event_loop()
    result_run: ResultSearch = loop.run_until_complete(
        search_phash_stream(phash_stream, distance)
    )
    console.print(result_run.timer)
