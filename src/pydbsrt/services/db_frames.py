from pathlib import Path
from typing import AsyncIterator, Optional, Tuple, Union

import asyncpg
from rich.progress import Progress

from pydbsrt.services.database import (
    console,
    create_conn,
    create_indexes_async,
    create_tables_async,
    error_console,
    psqlDbIpAddr,
    psqlDbName,
    psqlUserName,
    psqlUserPass,
)
from pydbsrt.services.models import PHashMedia
from pydbsrt.services.reader_frames import gen_read_binary_img_hash_file
from pydbsrt.tools.aio_filehash import aio_hashfile


async def import_binary_img_hash_to_db_async(
    binary_img_hash_file: Path, progress: Optional[Progress] = None
) -> Tuple[int, int]:
    media_hash = await aio_hashfile(binary_img_hash_file, hexdigest=True)
    console.print(f"media_hash='{media_hash}'")

    async with asyncpg.create_pool(
        user=psqlUserName, password=psqlUserPass, database=psqlDbName, host=psqlDbIpAddr, command_timeout=60
    ) as pool:
        async with pool.acquire() as conn:
            await create_tables_async(conn)
            await create_indexes_async(conn)

            # Test if this media is already in DB
            # https://magicstack.github.io/asyncpg/current/api/index.html?highlight=returning#asyncpg.connection.Connection.fetchval
            found_media_id = await conn.fetchval(
                """
                    SELECT
                        id
                    FROM
                        medias
                    WHERE
                        medias.media_hash = $1;
                """,
                media_hash,
            )
            if found_media_id:
                return found_media_id, 0

            # Insert Media
            media_id = await conn.fetchval(
                """
                    INSERT INTO
                        medias (media_hash, name)
                    VALUES
                        ($1, $2)
                    RETURNING
                        id;
                """,
                media_hash,
                binary_img_hash_file.stem,
            )
            if media_id is None:
                error_msg = (
                    f"Problem when inserting media (media_hash={media_hash}, name={binary_img_hash_file.stem}) into DB!"
                )
                error_console.print(error_msg)
                raise RuntimeError(error_msg)

            await conn.copy_records_to_table(
                "frames",
                records=gen_read_binary_img_hash_file(binary_img_hash_file, media_id, progress),
                columns=["p_hash", "frame_offset", "media_id"],
            )

            nb_frames_inserted = (
                await conn.fetchval(
                    """
                        SELECT
                            COUNT(*)
                        FROM
                            frames
                        WHERE
                            frames.media_id = $1;
                    """,
                    media_id,
                )
                or 0
            )
    return media_id, nb_frames_inserted


async def agen_p_hash_from_media_in_db(
    media_hash: Union[str, bytes],
    chunk_size: int = 32,
    limit: Optional[int] = None,
) -> AsyncIterator[PHashMedia]:
    conn = await create_conn()

    found_media_id: Optional[int] = await conn.fetchval(
        "SELECT id FROM medias WHERE medias.media_hash = $1", media_hash
    )
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
