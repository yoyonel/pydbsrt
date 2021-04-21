from pathlib import Path
from typing import Optional, Tuple, Union

import asyncpg
from asyncpg import Connection
from imohash import hashfile
from rich.progress import Progress

from pydbsrt.services.database import (
    psqlUserName,
    psqlUserPass,
    psqlDbName,
    psqlDbIpAddr,
    console,
    create_tables_async,
    create_indexes_async,
    error_console,
)
from pydbsrt.services.models import PHashMedia
from pydbsrt.services.reader_frames import gen_read_binary_img_hash_file


async def import_binary_img_hash_to_db_async(
    binary_img_hash_file: Path, progress: Optional[Progress] = None
) -> Tuple[int, int]:
    conn: Connection = await asyncpg.connect(
        user=psqlUserName, password=psqlUserPass, database=psqlDbName, host=psqlDbIpAddr
    )

    media_hash = hashfile(binary_img_hash_file, hexdigest=True)
    console.print(f"media_hash='{media_hash}'")

    # await drop_tables(conn)
    await create_tables_async(conn)
    await create_indexes_async(conn)

    # await reindex_tables(conn)

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
        error_console.print(
            f"Problem when inserting media (media_hash={media_hash}, name={binary_img_hash_file.stem}) into DB!"
        )
        raise RuntimeError("DB: Problem when inserting media")

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

    # await search_img_hash(conn)

    await conn.close()
    return media_id, nb_frames_inserted


async def agen_p_hash_from_media_in_db(
    # binary_img_hash_file: Path,
    media_hash: Union[str, bytes],
    chunk_size: int = 32,
    limit: Optional[int] = None,
):
    # media_hash = hashfile(binary_img_hash_file, hexdigest=True)
    # console.print(f"media_hash='{media_hash}'")

    conn: Connection = await asyncpg.connect(
        user=psqlUserName, password=psqlUserPass, database=psqlDbName, host=psqlDbIpAddr
    )

    found_media_id = await conn.fetchval(
        "SELECT id FROM medias WHERE medias.media_hash = $1", media_hash
    )
    if found_media_id is None:
        return

    async with conn.transaction():
        query = f"""
            SELECT
                p_hash, frame_offset
            FROM
                frames
            WHERE
                frames.media_id = $1
            ORDER BY
                frames.frame_offset
            {'LIMIT ' + str(limit) if limit else ''};
        """
        cur = await conn.cursor(query, found_media_id)
        records = await cur.fetch(chunk_size)
        while records:
            for record in records:
                yield PHashMedia(*record)
            records = await cur.fetch(chunk_size)