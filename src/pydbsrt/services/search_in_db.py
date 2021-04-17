from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, Set

import asyncpg
from asyncpg import Connection

from pydbsrt.applications import search_imghash_in_db
from pydbsrt.services.database import (
    psqlUserName,
    psqlUserPass,
    psqlDbName,
    psqlDbIpAddr,
)
from pydbsrt.services.reader_frames import build_reader_frames
from pydbsrt.tools.ffmpeg_tools.ffmeg_extract_frame import rawframe_to_imghash
from pydbsrt.tools.imghash import imghash_to_signed_int64


@dataclass(frozen=True)
class PairedMatchedFrame:
    record_search_offset: int
    match_frame_offset: int


@dataclass(frozen=True)
class BuildSearchResult:
    media_found: bool
    media_name_search: str
    media_name_match: str
    media_id_match: int
    nb_offsets_match: int
    search_offsets_match: Set[int]
    match_frames_offsets: Set[int]
    timer_in_seconds: float


async def search_media_in_db(
    search_media: Path, search_distance: int, nb_seconds_to_extract: float
) -> search_imghash_in_db.ResultSearch:
    reader, _ = build_reader_frames(
        search_media, nb_seconds_to_extract, seek_to_middle=True
    )
    gen_frame_hash = map(rawframe_to_imghash, reader)
    gen_signed_int64_hash = map(imghash_to_signed_int64, gen_frame_hash)
    return await search_imghash_in_db.search_phash_stream(
        map(str, gen_signed_int64_hash), search_distance
    )


async def search_media_name_into_db(conn: Connection, media_id: int) -> str:
    return await conn.fetchval(
        """SELECT "name"
            FROM "medias"
            WHERE medias.id = $1""",
        media_id,
    )


async def build_search_media_results(
    media: Path, results: search_imghash_in_db.ResultSearch
) -> List[BuildSearchResult]:
    map_media_id_to_offsets_matched: Dict[int, List[PairedMatchedFrame]] = defaultdict(
        list
    )
    for record in results.records:
        for match in record.matches:
            map_media_id_to_offsets_matched[match.media_id].append(
                PairedMatchedFrame(
                    record_search_offset=record.search_offset,
                    match_frame_offset=match.frame_offset,
                )
            )

    # https://magicstack.github.io/asyncpg/current/api/index.html#connection-pools
    async with asyncpg.create_pool(
        user=psqlUserName,
        password=psqlUserPass,
        database=psqlDbName,
        host=psqlDbIpAddr,
        command_timeout=60,
    ) as pool:
        async with pool.acquire() as conn:

            async def _build_result(
                media_id, paired_matched_frames
            ) -> BuildSearchResult:
                media_name_found = await search_media_name_into_db(conn, media_id)
                return BuildSearchResult(
                    media_found=media.stem == media_name_found,
                    media_name_search=media.stem,
                    media_id_match=media_id,
                    media_name_match=media_name_found,
                    nb_offsets_match=len(paired_matched_frames),
                    search_offsets_match={
                        paired_matched_frame.record_search_offset
                        for paired_matched_frame in paired_matched_frames
                    },
                    match_frames_offsets={
                        paired_matched_frame.match_frame_offset
                        for paired_matched_frame in paired_matched_frames
                    },
                    timer_in_seconds=results.timer.elapsed,
                )

            results = [
                await _build_result(media_id, paired_matched_frames)
                for media_id, paired_matched_frames in map_media_id_to_offsets_matched.items()
            ]
    return results
