from dataclasses import dataclass
from typing import List, Iterable

import asyncpg
from async_lru import alru_cache
from contexttimer import Timer

from pydbsrt.services.database import (
    psqlUserName,
    psqlUserPass,
    psqlDbName,
    psqlDbIpAddr,
)
from pydbsrt.tools.timer_profiling import _Timer


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


@alru_cache
async def search_phash_in_db(conn, phash: int, distance: int) -> List[MatchedFrame]:
    return [
        MatchedFrame(*record)
        for record in await conn.fetch(
            """
SELECT "frame_offset", "media_id"
FROM "frames"
WHERE "p_hash" <@ ($1, $2)""",
            phash,
            distance,
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
                # TODO: make generator here
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
