import asyncio
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import List

import asyncpg
import click
import click_pathlib
# from aiocache import cached
# from aiocache.serializers import PickleSerializer
from async_lru import alru_cache
from asyncpg import Connection
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


# @cached(ttl=10, serializer=PickleSerializer())
@alru_cache
async def search_img_hash(conn, search_phash: int, search_distance: int) -> List[MatchedFrame]:
    records = await conn.fetch(f"""
                SELECT "frame_offset", "media_id" 
                FROM "frames" 
                WHERE "p_hash" <@ ({search_phash}, {search_distance})""")
    return [MatchedFrame(*record) for record in records]


async def run(phash_stream, search_distance: int) -> ResultRun:
    records: List[ResultRecord] = []

    conn: Connection = await asyncpg.connect(user=psqlUserName, password=psqlUserPass, database=psqlDbName, host=psqlDbIpAddr)
    with _Timer("search_img_hash", func_to_log=lambda s: None) as timer:
        for search_offset, search_phash in enumerate(int(str_phash.rstrip()) for str_phash in phash_stream):
            matches = await search_img_hash(conn, search_phash, search_distance)
            records.append(ResultRecord(search_phash, search_offset, list(matches)))
    await conn.close()
    return ResultRun(records, timer)


@click.command(short_help="")
@click.option(
    "--phash_file", "-p",
    required=True,
    type=click_pathlib.Path(exists=True, readable=True, resolve_path=True, allow_dash=True),
    help="PHash stream to search in db"
)
@click.option(
    "--distance", "-d",
    default=1,
    type=int,
    help="Search distance to use"
)
def search_imghash_in_db(phash_file, distance):
    phash_stream = sys.stdin if phash_file == Path("-") else phash_file.open("r")
    loop = asyncio.get_event_loop()
    result_run: ResultRun = loop.run_until_complete(run(phash_stream, distance))
    console.print(result_run.timer)
