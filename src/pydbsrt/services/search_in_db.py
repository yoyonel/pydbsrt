from collections import defaultdict
from pathlib import Path
from typing import Dict, List

import asyncpg

from pydbsrt.models.matching import ResultSearch
from pydbsrt.models.searching import BuildSearchResult, PairedMatchedFrame
from pydbsrt.services.database import psqlDbIpAddr, psqlDbName, psqlUserName, psqlUserPass, search_media_name
from pydbsrt.services.matching import search_phash_stream_in_db
from pydbsrt.services.reader_frames import build_reader_frames
from pydbsrt.tools.ffmpeg_tools.ffmeg_extract_frame import rawframe_to_imghash
from pydbsrt.tools.imghash import imghash_to_signed_int64


async def search_media_in_db(
    search_media: Path,
    search_distance: int,
    nb_seconds_to_extract: float,
    seek_to_middle: bool = True,
) -> ResultSearch:
    reader, _ = build_reader_frames(search_media, nb_seconds_to_extract, seek_to_middle=seek_to_middle)
    gen_frame_hash = map(rawframe_to_imghash, reader)
    gen_signed_int64_hash = map(imghash_to_signed_int64, gen_frame_hash)
    return await search_phash_stream_in_db(map(str, gen_signed_int64_hash), search_distance)


async def build_search_media_results(media: Path, imghash_matched: ResultSearch) -> List[BuildSearchResult]:
    """

    Args:
        media:
        imghash_matched:

    Returns:

    """
    map_media_id_to_offsets_matched: Dict[int, List[PairedMatchedFrame]] = defaultdict(list)
    for record in imghash_matched.records:
        for match in record.matches:
            map_media_id_to_offsets_matched[match.media_id].append(
                PairedMatchedFrame(
                    record_search_offset=record.search_offset,
                    match_frame_offset=match.frame_offset,
                )
            )

    async def _build_result(media_id, paired_matched_frames) -> BuildSearchResult:
        media_name_found = await search_media_name(conn, media_id)

        nb_offsets_match = 0
        search_offsets_match: set[int] = set()
        match_frames_offsets: set[int] = set()
        for nb_offsets_match, paired_matched_frame in enumerate(paired_matched_frames, start=1):
            search_offsets_match.add(paired_matched_frame.record_search_offset)
            match_frames_offsets.add(paired_matched_frame.match_frame_offset)

        return BuildSearchResult(
            media_found=media.stem == media_name_found,
            media_name_search=media.stem,
            media_id_match=media_id,
            media_name_match=media_name_found,
            nb_offsets_match=nb_offsets_match,
            search_offsets_match=search_offsets_match,
            match_frames_offsets=match_frames_offsets,
            timer_in_seconds=imghash_matched.timer.elapsed,
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
            media_matched = [
                await _build_result(media_id, paired_matched_frames)
                for media_id, paired_matched_frames in map_media_id_to_offsets_matched.items()
            ]
    return media_matched


# async def search_img_hash(
#         conn, search_phash: int = -6023947298048657955, search_distance: int = 1
# ):
#     values = (
#             await conn.fetch(
#                 """
#                     SELECT "id", "p_hash", "frame_offset", "media_id"
#                     FROM "frames"
#                     WHERE "p_hash" <@ ($1, $2)""",
#                 search_phash,
#                 search_distance,
#             )
#             or []
#     )
#     console.print(
#         f"count(searching(phash={search_phash}, search_distance={search_distance}))={len(values)}"
#     )
