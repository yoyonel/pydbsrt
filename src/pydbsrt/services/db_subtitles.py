"""
ASYNC module
"""
import contextlib
from dataclasses import fields
from itertools import chain, groupby
from operator import itemgetter
from pathlib import Path
from typing import Iterator, List, Optional, Tuple

import asyncpg
from imohash import hashfile
from yaspin.core import Yaspin

from pydbsrt.models.database_import import ImportSubtitlesResult
from pydbsrt.models.subtitles import SubFrameRecord, SubtitlesRecord
from pydbsrt.services.database import (
    console,
    create_tables_for_subtitles,
    drop_tables,
    drop_types,
    error_console,
    psqlDbIpAddr,
    psqlDbName,
    psqlUserName,
    psqlUserPass,
    search_subframe_records_from_media,
    search_subtitles_from_hash,
)
from pydbsrt.services.insert_in_db import insert_subtitles_in_db
from pydbsrt.services.reader_frames import gen_read_binary_img_hash_file
from pydbsrt.tools.compare import compare_containers
from pydbsrt.tools.search_in_db import search_media_hash_in_db
from pydbsrt.tools.subfingerprint import SubFingerprint, SubFingerprints
from pydbsrt.tools.subreader import SubReader


async def import_subtitles_into_db(
    subtitles: Path,
    binary_img_hash_file: Path,
    spinner: Optional[Yaspin] = None,
    drop_before_inserting: bool = False,
    check_before_inserting: bool = False,
) -> ImportSubtitlesResult:
    """

    Args:
        subtitles:
        binary_img_hash_file:
        spinner:
        drop_before_inserting:
        check_before_inserting:

    Returns:

    """
    async with asyncpg.create_pool(
        user=psqlUserName,
        password=psqlUserPass,
        database=psqlDbName,
        host=psqlDbIpAddr,
        command_timeout=60,
    ) as pool:
        with spinner or contextlib.nullcontext() as spinner_subtitles:
            async with pool.acquire() as conn:
                if drop_before_inserting:
                    await drop_tables(conn, ("subtitles", "sub_frames"))
                    await drop_types(conn, ("LANG",))
                await create_tables_for_subtitles(conn)

                subtitles_hash = hashfile(subtitles, hexdigest=True)
                found_subtitles_id = await search_subtitles_from_hash(conn, subtitles_hash)
                nb_rows_inserted = 0
                if found_subtitles_id:
                    console.print(
                        f"Subtitles={subtitles.stem} with hash={subtitles_hash} already exist into db at subtitles_id={found_subtitles_id}"
                    )
                    return ImportSubtitlesResult(nb_rows_inserted)

                media_hash = hashfile(binary_img_hash_file, hexdigest=True)
                found_media_id: Optional[int] = await search_media_hash_in_db(conn, media_hash)
                if found_media_id is None:
                    console.print(f"ERROR: can't found media where media_hash={media_hash}")
                    return ImportSubtitlesResult(nb_rows_inserted)

                # insert subtitles into DB
                subtitles_id = await insert_subtitles_in_db(
                    conn, SubtitlesRecord(subtitles_hash, subtitles.stem, found_media_id)
                )
                if subtitles_id is None:
                    error_console.print(
                        f"Problem when inserting subtitles (subtitles_hash={subtitles_hash}, name={subtitles.stem}) into DB!"
                    )
                    raise RuntimeError("DB: Problem when inserting subtitles")
                nb_rows_inserted += 1

                it_img_hash: Iterator[int] = (
                    # TODO: maybe an error on last parameter `media_hash` (the signature of
                    #  `gen_read_binary_img_hash_file` reclaim a `media_id`)
                    img_hash
                    for img_hash, _, _ in gen_read_binary_img_hash_file(binary_img_hash_file, media_hash)
                )

                gb_sub_fingerprints: Iterator[Tuple[int, Iterator[SubFingerprint]]] = groupby(
                    SubFingerprints(sub_reader=SubReader(subtitles), imghash_reader=it_img_hash),
                    key=itemgetter("index"),
                )

                records_sub_frames: List[SubFrameRecord] = []
                for index_subtitle, it_indexed_sub_fingerprints in gb_sub_fingerprints:
                    if isinstance(index_subtitle, str):
                        index_subtitle = int(index_subtitle[1])

                    sub_fingerprint = next(it_indexed_sub_fingerprints)
                    it_indexed_sub_fingerprints = chain(
                        (SubFingerprint(index_subtitle, sub_fingerprint.offset_frame, sub_fingerprint.img_hash),),
                        it_indexed_sub_fingerprints,
                    )

                    if check_before_inserting:
                        end_frame_offset = await _check_import_subtitles_into_db(
                            conn,
                            it_indexed_sub_fingerprints,
                            sub_fingerprint.offset_frame,
                            found_media_id,
                        )
                    else:
                        end_frame_offset = sub_fingerprint.offset_frame + len(list(it_indexed_sub_fingerprints))

                    records_sub_frames.append(
                        SubFrameRecord(
                            index_subtitle,
                            sub_fingerprint.offset_frame,
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
    if spinner_subtitles:
        spinner_subtitles.ok("✅ ")
    return ImportSubtitlesResult(nb_rows_inserted)


async def _check_import_subtitles_into_db(
    conn,
    it_indexed_sub_fingerprints,
    start_frame_offset,
    found_media_id,
    raise_exception: bool = True,
) -> int:
    """

    Args:
        conn:
        it_indexed_sub_fingerprints:
        start_frame_offset:
        found_media_id:
        raise_exception:

    Returns:

    """
    ################################################################
    # DEBUG/VALIDITY PURPOSE
    ################################################################
    fingerprints = [fingerprint for _, _, fingerprint in it_indexed_sub_fingerprints]
    end_frame_offset = start_frame_offset + len(fingerprints)
    sub_nb_frames = end_frame_offset - start_frame_offset
    db_subframe_record = [
        subframe_search_result.p_hash
        async for subframe_search_result in search_subframe_records_from_media(
            conn, found_media_id, start_frame_offset, end_frame_offset
        )
    ]
    errors = compare_containers(fingerprints, db_subframe_record)
    if errors:
        console.print(f"sub_nb_frames={sub_nb_frames} - len(fingerprints)={len(fingerprints)}")
        console.print(errors)
        if raise_exception:
            raise RuntimeError("Inconsistencies when inserting subtitles !")
    ################################################################
    return end_frame_offset
