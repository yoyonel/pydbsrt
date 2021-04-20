"""
http://python-notes.curiousefficiency.org/en/latest/pep_ideas/async_programming.html#naming-conventions
"""
import contextlib
from itertools import groupby, chain
from operator import itemgetter
from pathlib import Path
from typing import Tuple, Optional, Union, Iterator

import asyncpg
from asyncpg import Connection
from imohash import hashfile
from rich.console import Console
from rich.progress import Progress
from yaspin.core import Yaspin

from pydbsrt import settings
from pydbsrt.services.models import PHashMedia, SubFrameRecord
from pydbsrt.services.reader_frames import gen_read_binary_img_hash_file
from pydbsrt.tools.compare import compare_containers
from pydbsrt.tools.subfingerprint import SubFingerprints
from pydbsrt.tools.subreader import SubReader

psqlDbIpAddr = settings.PSQL_IP
psqlDbName = settings.PSQL_DB_NAME
psqlUserName = settings.PSQL_USER
psqlUserPass = settings.PSQL_PASS

console = Console()
error_console = Console(stderr=True, style="bold red")


async def drop_tables_async(conn, tables_names=("medias",)):
    # https://docs.postgresql.fr/11/sql-droptable.html
    for table_name in tables_names:
        await conn.execute(f"DROP TABLE IF EXISTS {table_name} CASCADE;")


async def drop_types_async(conn, types_names=("LANG",)):
    # https://www.postgresql.org/docs/9.1/sql-droptype.html
    for type_name in types_names:
        await conn.execute(f"DROP TYPE IF EXISTS {type_name} CASCADE;")


async def create_tables_async(conn):
    await create_tables_for_medias_async(conn)
    await create_tables_for_frames_async(conn)
    await create_tables_for_subtitles_async(conn)


async def create_tables_for_medias_async(conn):
    """one-to-many: media ->* frames"""
    ################################################################
    # FIXME: media_hash has to be (fill with) the binary hash of the video file (not the binary imghash file)
    # TODO: maybe we need both: video file's and imghash file's (binary) hashes
    ################################################################
    await conn.execute(
        """
                CREATE TABLE IF NOT EXISTS medias (
                    id              SERIAL PRIMARY KEY,
                    media_hash      CHAR(32) UNIQUE,
                    -- https://www.postgresqltutorial.com/postgresql-char-varchar-text/
                    name            TEXT UNIQUE
                )"""
    )
    ################################################################


async def create_tables_for_frames_async(conn):
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


async def create_tables_for_subtitles_async(conn):
    ################################################################
    # Subtitles
    ################################################################
    # https://www.postgresql.org/docs/9.1/datatype-enum.html#:~:text=Enumerated%20(enum)%20types%20are%20data,for%20a%20piece%20of%20data.
    await conn.execute(
        """
        /* https://stackoverflow.com/a/48382296 */
        DO $$ BEGIN
            CREATE TYPE LANG AS ENUM ('FR', 'EN');
        EXCEPTION
            WHEN duplicate_object THEN null;
        END $$;

        CREATE TABLE IF NOT EXISTS subtitles (
            id              SERIAL PRIMARY KEY,
            /* https://www.postgresql.org/docs/9.4/ddl-constraints.html */
            subtitles_hash  CHAR(32) NOT NULL,
            name            TEXT NOT NULL,
            lang            LANG DEFAULT 'FR',
            media_id        INT NOT NULL,
            FOREIGN KEY (media_id) REFERENCES medias(id) ON DELETE CASCADE,
            UNIQUE(subtitles_hash, name, lang)
        );

        CREATE TABLE IF NOT EXISTS sub_frames (
            id                  SERIAL PRIMARY KEY,
            index               INT NOT NULL,
            start_frame_offset  INT NOT NULL,
            end_frame_offset    INT NOT NULL,
            text                TEXT DEFAULT '',
            subtitles_id        INT NOT NULL,
            FOREIGN KEY (subtitles_id) REFERENCES subtitles(id) ON DELETE CASCADE,
            CHECK (end_frame_offset >= start_frame_offset)
        );
        """
    )
    ################################################################


async def create_indexes_async(conn):
    await conn.execute(
        """
            CREATE INDEX IF NOT EXISTS
                frames_phash_bk_tree_index
            ON
                frames
            USING
                spgist ( p_hash bktree_ops );
        """
    )


async def reindex_tables_async(conn):
    """
    https://www.postgresql.org/docs/9.4/sql-reindex.html
    """
    await conn.execute(
        """
            REINDEX TABLE medias;
            REINDEX TABLE frames;
            REINDEX TABLE subtitles;
        """
    )


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


async def _import_subtitles_into_db_check_async(
    conn,
    it_indexed_sub_fingerprints,
    start_frame_offset,
    found_media_id,
    raise_exception: bool = True,
):
    ################################################################
    # DEBUG/VALIDITY PURPOSE
    ################################################################
    fingerprints = [fingerprint for _, _, fingerprint in it_indexed_sub_fingerprints]
    end_frame_offset = start_frame_offset + len(fingerprints)
    sub_nb_frames = end_frame_offset - start_frame_offset
    records = await conn.fetch(
        """
            SELECT
                id, p_hash, frame_offset
            FROM
                frames
            WHERE
                frames.media_id = $1 AND
                frames.frame_offset >= $2 AND
                frames.frame_offset < $3
            LIMIT $4;
        ;""",
        found_media_id,
        start_frame_offset,
        end_frame_offset,
        sub_nb_frames,
    )
    db_frame_id_p_hash = [
        (db_frame_id, db_frame_p_hash, db_frame_offset)
        for db_frame_id, db_frame_p_hash, db_frame_offset in records
    ]
    db_frame_p_hash = [p_hash for _, p_hash, _ in db_frame_id_p_hash]
    errors = compare_containers(fingerprints, db_frame_p_hash)
    if errors:
        console.print(
            f"sub_nb_frames={sub_nb_frames} - len(fingerprints)={len(fingerprints)}"
        )
        console.print(errors)
        if raise_exception:
            raise RuntimeError("Inconsistencies when inserting subtitles !")
    ################################################################
    return end_frame_offset


async def import_subtitles_into_db_async(
    subtitles: Path,
    binary_img_hash_file: Path,
    spinner: Optional[Yaspin] = None,
    drop_before_inserting: bool = False,
    check_before_inserting: bool = False,
) -> int:
    async with asyncpg.create_pool(
        user=psqlUserName,
        password=psqlUserPass,
        database=psqlDbName,
        host=psqlDbIpAddr,
        command_timeout=60,
    ) as pool:
        with (spinner or contextlib.nullcontext()) as spinner_subtitles:
            async with pool.acquire() as conn:
                if drop_before_inserting:
                    await drop_tables_async(conn, ("subtitles", "sub_frames"))
                    await drop_types_async(conn, ("LANG",))
                await create_tables_for_subtitles_async(conn)

                subtitles_hash = hashfile(subtitles, hexdigest=True)
                found_subtitles_id = await conn.fetchval(
                    """
                        SELECT
                            id
                        FROM
                            subtitles
                        WHERE
                            subtitles.subtitles_hash = $1;
                    """,
                    subtitles_hash,
                )
                nb_rows_inserted = 0
                if found_subtitles_id:
                    console.print(
                        f"Subtitles={subtitles.stem} with hash={subtitles_hash} already exist into db at subtitles_id={found_subtitles_id}"
                    )
                    return nb_rows_inserted

                media_hash = hashfile(binary_img_hash_file, hexdigest=True)
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
                if found_media_id is None:
                    console.print(
                        f"ERROR: can't found media where media_hash={media_hash}"
                    )
                    return nb_rows_inserted

                # insert subtitles into DB
                subtitles_id = await conn.fetchval(
                    """
                        INSERT INTO
                            subtitles (subtitles_hash, name, media_id)
                        VALUES
                            ($1, $2, $3)
                        RETURNING
                            id;
                    """,
                    subtitles_hash,
                    subtitles.stem,
                    found_media_id,
                )
                if subtitles_id is None:
                    error_console.print(
                        f"Problem when inserting subtitles (subtitles_hash={subtitles_hash}, name={subtitles.stem}) into DB!"
                    )
                    raise RuntimeError("DB: Problem when inserting subtitles")
                nb_rows_inserted += 1

                it_img_hash: Iterator[int] = (
                    img_hash
                    for img_hash, _, _ in gen_read_binary_img_hash_file(
                        binary_img_hash_file, media_hash
                    )
                )

                gb_sub_fingerprints = groupby(
                    SubFingerprints(
                        sub_reader=SubReader(subtitles), imghash_reader=it_img_hash
                    ),
                    key=itemgetter("index"),
                )

                records_sub_frames = []
                for index_subtitle, it_indexed_sub_fingerprints in gb_sub_fingerprints:
                    if isinstance(index_subtitle, str):
                        index_subtitle = int(index_subtitle[1])

                    _, start_frame_offset, fingerprint = next(
                        it_indexed_sub_fingerprints
                    )
                    it_indexed_sub_fingerprints = chain(
                        ((index_subtitle, start_frame_offset, fingerprint),),
                        it_indexed_sub_fingerprints,
                    )

                    if check_before_inserting:
                        end_frame_offset = await _import_subtitles_into_db_check_async(
                            conn,
                            it_indexed_sub_fingerprints,
                            start_frame_offset,
                            found_media_id,
                        )
                    else:
                        end_frame_offset = start_frame_offset + len(
                            list(it_indexed_sub_fingerprints)
                        )

                    records_sub_frames.append(
                        SubFrameRecord(
                            index_subtitle,
                            start_frame_offset,
                            end_frame_offset,
                            "",
                            subtitles_id,
                        )
                    )
                # https://magicstack.github.io/asyncpg/current/api/index.html#asyncpg.connection.Connection.copy_records_to_table
                # https://www.postgresql.org/docs/current/sql-copy.html
                result = await conn.copy_records_to_table(
                    "sub_frames",
                    records=records_sub_frames,
                    columns=[
                        "index",
                        "start_frame_offset",
                        "end_frame_offset",
                        "text",
                        "subtitles_id",
                    ],
                )
                # On successful completion,
                # a COPY command returns a command tag of the form: "COPY count"
                nb_rows_inserted += int(result.split()[1])
    if spinner:
        spinner_subtitles.ok("✅ ")
    return nb_rows_inserted
