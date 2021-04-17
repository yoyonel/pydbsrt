from pathlib import Path
from typing import Tuple

import asyncpg
from asyncpg import Connection
from imohash import hashfile
from rich.console import Console

from pydbsrt import settings
from pydbsrt.services.reader_frames import gen_read_binary_img_hash_file

psqlDbIpAddr = settings.PSQL_IP
psqlDbName = settings.PSQL_DB_NAME
psqlUserName = settings.PSQL_USER
psqlUserPass = settings.PSQL_PASS

console = Console()


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
    await conn.execute(
        """
                CREATE TABLE IF NOT EXISTS srt (
                    id              SERIAL PRIMARY KEY,
                    index           INT,
                    media_id        INT,
                    start_frame_id  INT,
                    end_frame_id    INT,
                    FOREIGN KEY (media_id) REFERENCES medias(id) ON DELETE CASCADE,
                    FOREIGN KEY (start_frame_id) REFERENCES frames(id) ON DELETE CASCADE,
                    FOREIGN KEY (end_frame_id) REFERENCES frames(id) ON DELETE CASCADE,
                    CHECK (end_frame_id >= start_frame_id)
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


async def reindex_tables(conn):
    await conn.execute(
        """
                REINDEX TABLE medias;
                REINDEX TABLE frames;
                REINDEX TABLE srt;
        """
    )


async def import_binary_img_hash_to_db(
    binary_img_hash_file: Path, progress
) -> Tuple[int, int]:
    """"""
    media_hash = hashfile(binary_img_hash_file, hexdigest=True)
    console.print(f"media_hash='{media_hash}'")

    conn: Connection = await asyncpg.connect(
        user=psqlUserName, password=psqlUserPass, database=psqlDbName, host=psqlDbIpAddr
    )

    # await drop_tables(conn)
    await create_tables(conn)
    await create_indexes(conn)

    # await reindex_tables(conn)

    # Test if this media is already in DB
    # https://magicstack.github.io/asyncpg/current/api/index.html?highlight=returning#asyncpg.connection.Connection.fetchval
    found_media_id = await conn.fetchval(
        f"SELECT id FROM medias WHERE medias.media_hash = '{media_hash}'"
    )
    if found_media_id:
        return found_media_id, 0

    # Insert Media
    media_id = await conn.fetchval(
        """
            INSERT INTO medias (media_hash, name)
            VALUES ($1, $2) RETURNING id""",
        media_hash,
        binary_img_hash_file.stem,
    )

    await conn.copy_records_to_table(
        "frames",
        records=gen_read_binary_img_hash_file(binary_img_hash_file, media_id, progress),
        columns=["p_hash", "frame_offset", "media_id"],
    )

    nb_frames_inserted = await conn.fetchval(
        f"SELECT COUNT(*) FROM frames WHERE frames.media_id = {media_id}",
    )

    # await search_img_hash(conn)

    await conn.close()
    return media_id, nb_frames_inserted
