import asyncio
from pathlib import Path
from typing import Iterator, Tuple

import asyncpg
import click
import click_pathlib
import numpy as np
from asyncpg import Connection
from imohash import hashfile
from rich.console import Console
from rich.progress import (
    BarColumn,
    DownloadColumn,
    TextColumn,
    TransferSpeedColumn,
    Progress, )

from pydbsrt import settings
from pydbsrt.tools.imghash import binary_to_signed_int64
from pydbsrt.tools.rich_colums import TimeElapsedOverRemainingColumn

psqlDbIpAddr = settings.PSQL_IP
psqlDbName = settings.PSQL_DB_NAME
psqlUserName = settings.PSQL_USER
psqlUserPass = settings.PSQL_PASS

console = Console()

progress = Progress(
    TextColumn("[bold blue]{task.fields[filename]}", justify="right"),
    BarColumn(bar_width=None),
    "[progress.percentage]{task.percentage:>3.1f}%",
    "•",
    DownloadColumn(),
    "•",
    TransferSpeedColumn(),
    "•",
    TimeElapsedOverRemainingColumn(),
    console=console
)


async def drop_tables(conn):
    for table_name in ("frames", "medias"):
        await conn.execute(f"""DROP TABLE IF EXISTS {table_name} CASCADE;""")


async def create_tables(conn):
    """
    one-to-many: media ->* frames
    """
    await conn.execute(f"""
                CREATE TABLE IF NOT EXISTS medias (
                    id              SERIAL PRIMARY KEY,
                    media_hash      CHAR(32) UNIQUE,
                    -- https://www.postgresqltutorial.com/postgresql-char-varchar-text/
                    name            TEXT
                )""")
    await conn.execute(f"""
                CREATE TABLE IF NOT EXISTS frames (
                    id              SERIAL PRIMARY KEY,
                    p_hash          BIGINT,
                    frame_offset    INT,
                    media_id        INT,
                    FOREIGN KEY (media_id) REFERENCES medias(id) ON DELETE CASCADE
                )""")


async def create_indexes(conn):
    await conn.execute(f"""
                CREATE INDEX IF NOT EXISTS frames_phash_bk_tree_index ON frames 
                USING spgist ( p_hash bktree_ops );""")


async def search_img_hash(conn, search_phash=-6023947298048657955, search_distance=1):
    values = await conn.fetch(f"""
                SELECT "id", "p_hash", "frame_offset", "media_id" 
                FROM "frames" 
                WHERE "p_hash" <@ ({search_phash}, {search_distance})""")
    console.print(f"count(searching(phash={search_phash}, search_distance={search_distance}))={len(values)}")


async def run(binary_img_hash_file: Path):
    media_hash = hashfile(binary_img_hash_file, hexdigest=True)
    console.print(f"media_hash='{media_hash}'")

    conn: Connection = await asyncpg.connect(user=psqlUserName, password=psqlUserPass, database=psqlDbName, host=psqlDbIpAddr)

    # await drop_tables(conn)
    await create_tables(conn)
    await create_indexes(conn)

    # Test if this media is already in DB
    # https://magicstack.github.io/asyncpg/current/api/index.html?highlight=returning#asyncpg.connection.Connection.fetchval
    found_media_id = await conn.fetchval(f"""
            SELECT id FROM medias WHERE medias.media_hash = '{media_hash}' 
                """)
    if found_media_id:
        console.print(f"binary_img_hash_file={str(binary_img_hash_file)} already in DB (medias.id={found_media_id})")
        return

    await create_tables(conn)
    await create_indexes(conn)

    # Insert Media
    media_id = await conn.fetchval(f"""
            INSERT INTO medias (media_hash, name)
            VALUES ($1, $2) RETURNING id""", media_hash, binary_img_hash_file.stem)

    with binary_img_hash_file.open("rb") as fo:
        task_id = progress.add_task("insert img_hash into db", filename=binary_img_hash_file.name, start=True)
        progress.update(task_id, total=binary_img_hash_file.stat().st_size // 8)

        def gen_records() -> Iterator[Tuple]:
            with progress:
                # TODO: maybe trying to read more bytes (chunk) to optimize (need to profile)
                ba_img_hex = fo.read(8)
                offset_frame = 0
                while ba_img_hex:
                    yield binary_to_signed_int64(ba_img_hex), offset_frame, media_id
                    ba_img_hex = fo.read(8)
                    offset_frame += 1
                    progress.update(task_id, advance=1, refresh=False)

        await conn.copy_records_to_table("frames", records=gen_records(), columns=["p_hash", "frame_offset", "media_id"])

        values = await conn.fetch(
            f"SELECT COUNT(*) FROM frames WHERE frames.media_id = {media_id}",
        )
        console.print(f"count(frames)={values}")

        await search_img_hash(conn)

    await conn.close()


@click.command(short_help="")
@click.option(
    "--binary_img_hash_file", "-r",
    required=True,
    type=click_pathlib.Path(exists=True, readable=True, resolve_path=True, allow_dash=False),
    help="Path to media"
)
def import_images_hashes_into_db(binary_img_hash_file):
    loop = asyncio.get_event_loop()
    loop.run_until_complete(run(binary_img_hash_file))
