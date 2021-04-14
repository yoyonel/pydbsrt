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
from dataclasses import dataclass
from pathlib import Path
from typing import List, Iterable

import asyncpg
import click
import click_pathlib
from async_lru import alru_cache
from contexttimer import Timer
from rich.console import Console

from pydbsrt import settings
from pydbsrt.tools.timer_profiling import _Timer

psqlDbIpAddr = settings.PSQL_IP
psqlDbName = settings.PSQL_DB_NAME
psqlUserName = settings.PSQL_USER
psqlUserPass = settings.PSQL_PASS

console = Console()


@dataclass(frozen=True)
class MatchedFrame:
    frame_offset: int
    media_id: int


@dataclass(frozen=True)
class ResultRecord:
    search_phash: int
    search_offset: int
    matches: List[MatchedFrame]


@dataclass(frozen=True)
class ResultRun:
    records: List[ResultRecord]
    timer: Timer


@alru_cache
async def search_phash_in_db(conn, phash: int, distance: int) -> List[MatchedFrame]:
    return [
        MatchedFrame(*record)
        for record in await conn.fetch(
            f"""
SELECT "frame_offset", "media_id"
FROM "frames"
WHERE "p_hash" <@ ({phash}, {distance})"""
        )
    ]


async def search_phash_stream(
    phash_stream: Iterable[str], search_distance: int
) -> ResultRun:
    """"""
    async with asyncpg.create_pool(
        user=psqlUserName,
        password=psqlUserPass,
        database=psqlDbName,
        host=psqlDbIpAddr,
        command_timeout=60,
    ) as pool:
        async with pool.acquire() as conn:
            with _Timer("search_img_hash", func_to_log=lambda s: None) as timer:
                # PEP 530 -- Asynchronous Comprehensions: https://www.python.org/dev/peps/pep-0530/
                records = [
                    ResultRecord(
                        phash,
                        offset,
                        await search_phash_in_db(conn, phash, search_distance),
                    )
                    for offset, phash in enumerate(
                        int(str_phash.rstrip()) for str_phash in phash_stream
                    )
                ]
    return ResultRun(records, timer)


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
    result_run: ResultRun = loop.run_until_complete(
        search_phash_stream(phash_stream, distance)
    )
    console.print(result_run.timer)
