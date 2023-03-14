from typing import Optional

from asyncpg import Connection

from pydbsrt.models.subtitles import SubtitlesRecord


async def insert_subtitles_in_db(conn: Connection, srt_in_db: SubtitlesRecord) -> Optional[int]:
    subtitles_id: Optional[int] = await conn.fetchval(
        """
            INSERT INTO
                subtitles (subtitles_hash, name, media_id)
            VALUES
                ($1, $2, $3)
            RETURNING
                id;
        """,
        srt_in_db.subtitles_hash,
        srt_in_db.name,
        srt_in_db.media_id,
    )

    return subtitles_id
