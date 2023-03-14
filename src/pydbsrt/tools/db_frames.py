from typing import AsyncIterator, Optional, Union

from pydbsrt.models.phash import PHashMedia
from pydbsrt.services.database import create_conn
from pydbsrt.tools.search_in_db import search_media_hash_in_db


async def agen_p_hash_from_media_in_db(
    media_hash: Union[str, bytes],
    chunk_size: int = 32,
    limit: Optional[int] = None,
) -> AsyncIterator[PHashMedia]:
    conn = await create_conn()

    found_media_id: Optional[int] = await search_media_hash_in_db(conn, media_hash)
    if found_media_id is None:
        return

    async with conn.transaction():
        # https://www.postgresql.org/docs/8.1/queries-limit.html
        query = """
            SELECT
                p_hash, frame_offset
            FROM
                frames
            WHERE
                frames.media_id = $1
            ORDER BY
                frames.frame_offset
        """
        query += f"\n{f'LIMIT {limit}' if limit else ''};"
        cur = await conn.cursor(query, found_media_id)
        while records := await cur.fetch(chunk_size):
            for record in records:
                yield PHashMedia(*record)
