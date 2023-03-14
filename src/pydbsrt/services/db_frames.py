from pathlib import Path
from typing import Optional

import asyncpg
from rich.progress import Progress

from pydbsrt.models.database_import import ImportBinaryImageHashResult
from pydbsrt.services.database import (
    console,
    create_indexes,
    create_tables,
    error_console,
    psqlDbIpAddr,
    psqlDbName,
    psqlUserName,
    psqlUserPass,
)
from pydbsrt.services.reader_frames import gen_read_binary_img_hash_file
from pydbsrt.tools.aio_filehash import hashfile
from pydbsrt.tools.search_in_db import search_media_hash_in_db


async def import_binary_img_hash_to_db_async(
    binary_img_hash_file: Path, progress: Optional[Progress] = None
) -> ImportBinaryImageHashResult:
    media_hash = await hashfile(binary_img_hash_file, hexdigest=True)
    console.print(f"media_hash='{media_hash}'")

    async with asyncpg.create_pool(
        user=psqlUserName, password=psqlUserPass, database=psqlDbName, host=psqlDbIpAddr, command_timeout=60
    ) as pool:
        async with pool.acquire() as conn:
            await create_tables(conn)
            await create_indexes(conn)

            # Test if this media is already in DB
            found_media_id = await search_media_hash_in_db(conn, media_hash)
            if found_media_id:
                return ImportBinaryImageHashResult(found_media_id, 0)

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
    return ImportBinaryImageHashResult(media_id, nb_frames_inserted)
