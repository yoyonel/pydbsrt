# -*- coding: utf-8 -*-
"""
➜ poetry run python src/pydbsrt/app_cli.py recognize-media --help
Usage: app_cli.py recognize-media [OPTIONS]

Options:
  -m, --media PATH                Media to recognize  [required]
  -d, --search_distance INTEGER   Search distance to use
  -s, --nb_seconds_to_extract FLOAT
                                  Nb seconds to use for cutting the media and
                                  searching

  --output_format [DataFrame|CSV]
  -h, --help                      Show this message and exit.

# Description
Application to launch a search from media (video) on BKTreeDB (implement on PostgreSQL database) and printout matches

# Example
```sh
find "/home/latty/NAS/tvshow/Silicon Valley/" -type f -name "*.mkv" | \
xargs -I {} python app_cli.py recognize-media -m "{}" --output_format CSV
[...]
The frame size for reading (32, 32) is different from the source frame size (1920, 1080).
True,Silicon.Valley.S05E08.Fifty-One.Percent.1080p.AMZN.WEB-DL.DD.5.1.H.265-SiGMA,Silicon.Valley.S05E08.Fifty-One.Percent.1080p.AMZN.WEB-DL.DD.5.1.H.265-SiGMA,38,33,"{0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23}","{21891, 21892, 21893, 21894, 21895, 21896, 21897, 21898, 21899, 21900, 21901, 21902, 21903, 21904, 21905, 21906, 21907, 21908, 21909, 21910,
21911, 21912, 21913, 21914}",0.31966724100129795
```

"""
from dataclasses import asdict
from enum import Enum
from pathlib import Path
from typing import List, Optional, Union

import click
import click_pathlib
import pandas as pd
from loguru import logger
from rich.console import Console

from pydbsrt.applications.search_imghash_in_db import ResultSearch
from pydbsrt.services.search_in_db import BuildSearchResult, build_search_media_results, search_media_in_db
from pydbsrt.tools.coro import coroclick

console = Console()

# https://docs.python.org/3.11/howto/enum.html#functional-api
OUTPUT_FORMAT = Enum("output_format", "DataFrame CSV")


@click.command(short_help="")
@click.option(
    "--media",
    "-m",
    required=True,
    type=click_pathlib.Path(exists=True, readable=True, resolve_path=True, allow_dash=True),
    help="Media to recognize",
)
@click.option("--search_distance", "-d", default=1, type=int, help="Search distance to use")
@click.option(
    "--nb_seconds_to_extract",
    "-s",
    default=1.00,
    type=float,
    help="Nb seconds to use for cutting the media and searching",
)
@click.option(
    "--output_format",
    type=click.Choice(list(map(lambda x: x.name, OUTPUT_FORMAT)), case_sensitive=False),
    default=OUTPUT_FORMAT.DataFrame.name,
)
@coroclick
@logger.catch
async def recognize_media(
    media: Path,
    search_distance: int,
    nb_seconds_to_extract: float,
    output_format: OUTPUT_FORMAT,
):
    results_from_search_imghash_in_db: ResultSearch = await search_media_in_db(
        media, search_distance, nb_seconds_to_extract
    )
    results: List[BuildSearchResult] = await build_search_media_results(media, results_from_search_imghash_in_db)

    # https://github.com/pandas-dev/pandas/issues/21910#issuecomment-405109225
    df_results = pd.DataFrame([asdict(x) for x in results])
    results_to_print: Union[pd.Dataframe, Optional[str]] = df_results
    if output_format == OUTPUT_FORMAT.CSV.name:
        results_to_print = df_results.to_csv(index=False, header=False)
    console.print(results_to_print)
