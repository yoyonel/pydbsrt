from dataclasses import dataclass
from typing import List, Iterable

import asyncpg
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
class ResultSearchRecord:
    search_phash: int
    search_offset: int
    matches: List[MatchedFrame]


@dataclass(frozen=True)
class ResultSearch:
    records: List[ResultSearchRecord]
    timer: Timer


async def drop_tables(conn):
    for table_name in ("frames", "medias"):
        await conn.execute(f"""DROP TABLE IF EXISTS {table_name} CASCADE;""")


async def create_tables(conn):
    """
    one-to-many: media ->* frames
    """
    await conn.execute(
        """
                CREATE TABLE IF NOT EXISTS medias (
                    id              SERIAL PRIMARY KEY,
                    media_hash      CHAR(32) UNIQUE,
                    -- https://www.postgresqltutorial.com/postgresql-char-varchar-text/
                    name            TEXT
                )"""
    )
    await conn.execute(
        """
                CREATE TABLE IF NOT EXISTS frames (
                    id              SERIAL PRIMARY KEY,
                    p_hash          BIGINT,
                    frame_offset    INT,
                    media_id        INT,
                    FOREIGN KEY (media_id) REFERENCES medias(id) ON DELETE CASCADE
                )"""
    )


async def create_indexes(conn):
    await conn.execute(
        """
                CREATE INDEX IF NOT EXISTS frames_phash_bk_tree_index ON frames
                USING spgist ( p_hash bktree_ops );"""
    )


async def search_img_hash(conn, search_phash=-6023947298048657955, search_distance=1):
    values = await conn.fetch(
        f"""
                SELECT "id", "p_hash", "frame_offset", "media_id"
                FROM "frames"
                WHERE "p_hash" <@ ({search_phash}, {search_distance})"""
    )
    console.print(
        f"count(searching(phash={search_phash}, search_distance={search_distance}))={len(values)}"
    )


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
) -> ResultSearch:
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
                    ResultSearchRecord(
                        phash,
                        offset,
                        await search_phash_in_db(conn, phash, search_distance),
                    )
                    for offset, phash in enumerate(
                        int(str_phash.rstrip()) for str_phash in phash_stream
                    )
                ]
    return ResultSearch(records, timer)
