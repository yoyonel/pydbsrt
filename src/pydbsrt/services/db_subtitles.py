import contextlib
from dataclasses import fields
from itertools import groupby, chain
from operator import itemgetter
from pathlib import Path
from typing import Optional, Iterator, List

import asyncpg
from imohash import hashfile
from yaspin.core import Yaspin

from pydbsrt.services.database import (
    psqlUserName,
    psqlUserPass,
    psqlDbName,
    psqlDbIpAddr,
    drop_tables_async,
    drop_types_async,
    create_tables_for_subtitles_async,
    console,
    error_console,
)
from pydbsrt.services.models import SubFrameRecord
from pydbsrt.services.reader_frames import gen_read_binary_img_hash_file
from pydbsrt.tools.compare import compare_containers
from pydbsrt.tools.subfingerprint import SubFingerprints
from pydbsrt.tools.subreader import SubReader


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

                records_sub_frames: List[SubFrameRecord] = []
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
                            text="",
                            subtitles_id=subtitles_id,
                        )
                    )
                # https://magicstack.github.io/asyncpg/current/api/index.html#asyncpg.connection.Connection.copy_records_to_table
                # https://www.postgresql.org/docs/current/sql-copy.html
                result = await conn.copy_records_to_table(
                    "sub_frames",
                    records=records_sub_frames,
                    columns=(field.name for field in fields(SubFrameRecord)),
                )
                # On successful completion,
                # a COPY command returns a command tag of the form: "COPY count"
                nb_rows_inserted += int(result.split()[1])
    if spinner:
        spinner_subtitles.ok("✅ ")
    return nb_rows_inserted


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