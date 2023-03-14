"""
ASYNC module

- http://python-notes.curiousefficiency.org/en/latest/pep_ideas/async_programming.html#naming-conventions
- https://peps.python.org/pep-0492/#why-magic-methods-start-with-a
"""
from typing import AsyncIterator, Optional

import asyncpg
from asyncpg import Connection
from rich.console import Console

from pydbsrt import settings
from pydbsrt.models.phash import Hash
from pydbsrt.models.searching import SubFrameSearchResult

psqlDbIpAddr = settings.PSQL_IP
psqlDbName = settings.PSQL_DB_NAME
psqlUserName = settings.PSQL_USER
psqlUserPass = settings.PSQL_PASS

console = Console()
error_console = Console(stderr=True, style="bold red")


async def create_conn() -> Connection:
    conn: Connection = await asyncpg.connect(
        user=psqlUserName, password=psqlUserPass, database=psqlDbName, host=psqlDbIpAddr
    )
    return conn


async def drop_tables(conn, tables_names=("medias",)) -> None:
    # https://docs.postgresql.fr/11/sql-droptable.html
    for table_name in tables_names:
        await conn.execute(f"DROP TABLE IF EXISTS {table_name} CASCADE;")


async def drop_types(conn, types_names=("LANG",)) -> None:
    # https://www.postgresql.org/docs/9.1/sql-droptype.html
    for type_name in types_names:
        await conn.execute(f"DROP TYPE IF EXISTS {type_name} CASCADE;")


async def create_tables(conn) -> None:
    await create_tables_for_medias(conn)
    await create_tables_for_frames(conn)
    await create_tables_for_subtitles(conn)


async def create_tables_for_medias(conn) -> None:
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


async def create_tables_for_frames(conn) -> None:
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


async def create_tables_for_subtitles(conn) -> None:
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


async def create_indexes(conn) -> None:
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


async def reindex_tables(conn) -> None:
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


async def search_media_name(conn: Connection, media_id: int) -> str:
    return await conn.fetchval(
        """SELECT "name"
            FROM "medias"
            WHERE medias.id = $1""",
        media_id,
    )


async def search_subtitles_from_hash(conn: Connection, subtitles_hash: Hash) -> Optional[int]:
    """

    Args:
        conn:
        subtitles_hash:

    Returns:

    """
    found_subtitles_id: Optional[int] = await conn.fetchval(
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
    return found_subtitles_id


async def search_subframe_records_from_media(
    conn: Connection,
    search_media_id: int,
    start_frame_offset: int,
    end_frame_offset: int,
) -> AsyncIterator[SubFrameSearchResult]:
    """

    Args:
        conn:
        search_media_id:
        start_frame_offset:
        end_frame_offset:

    Returns:

    """
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
        search_media_id,
        start_frame_offset,
        end_frame_offset,
        end_frame_offset - start_frame_offset,
    )
    for record in records:
        yield SubFrameSearchResult(*record)
