# -*- coding: utf-8 -*-
import asyncio
import datetime
from collections import defaultdict
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Dict, Iterator, List, Set

import asyncpg
import click
import click_pathlib
import pandas as pd
from asyncpg import Connection
from imageio_ffmpeg import read_frames
from rich.console import Console

from pydbsrt import settings
from pydbsrt.services import search_imghash_in_db
from pydbsrt.tools.ffmpeg_tools.ffmeg_extract_frame import rawframe_to_imghash
from pydbsrt.tools.imghash import imghash_to_signed_int64

psqlDbIpAddr = settings.PSQL_IP
psqlDbName = settings.PSQL_DB_NAME
psqlUserName = settings.PSQL_USER
psqlUserPass = settings.PSQL_PASS

console = Console()


@dataclass(frozen=True)
class PairedMatchedFrame:
    record_search_offset: int
    match_frame_offset: int


@dataclass(frozen=True)
class BuildResult:
    media_found: bool
    media_name_search: str
    media_name_match: str
    media_id_match: int
    nb_offsets_match: int
    search_offsets_match: Set[int]
    match_frames_offsets: Set[int]
    timer_in_seconds: float


def build_iframe_selection(pict_type: str = "I") -> str:
    """
    #######################################################################
    # I-FRAME
    #######################################################################
    Selection des I-Frames du cut de la video
    attention: toute les frames du cut sont envoyées avec répétition des I-Frames
    par exemple, les frames du médias sont: (* pour les I-FRAMES)
      Frame_0*    Frame_1     Frame_2     Frame_3*    Frame_4
    alors la sélection produira les frames suivantes:
      Frame_0*    Frame_0     Frame_0     Frame_3*    Frame_3
    attention 2: selectionner les I-FRAMES semblent plus lent (FFMPEG) ...
    faudrait mettre en place l'économie des frames intermédiaires pour évaluer l'optimisation/économie
    """
    return f"select=eq(pict_type\\,{pict_type})"


def build_reader_frames(media: Path, nb_seconds_to_extract: float) -> Iterator[bytes]:
    # Read a video file
    meta = next(read_frames(media))
    # extract a (frame's) chunk around/in middle of the media
    # https://trac.ffmpeg.org/wiki/Seeking#Cuttingsmallsections
    ffmpeg_seek_input_cmd = [
        "-ss",
        str(datetime.timedelta(seconds=meta["duration"] // 2)),
    ]
    ffmpeg_seek_output_cmd = [
        "-frames:v",
        str(round(nb_seconds_to_extract * meta["fps"])),
    ]
    # ffmpeg_seek_output_cmd = ["-to", str(datetime.timedelta(seconds=nb_seconds_to_extract))]
    video_filters = ", ".join(
        (
            # build_iframe_selection(),
            # Redimensionnement des frames output en 32x32
            "scale=width=32:height=32",
        )
    )
    reader = read_frames(
        media,
        input_params=[*ffmpeg_seek_input_cmd],
        output_params=[
            *ffmpeg_seek_output_cmd,
            "-vf",
            video_filters,
            "-pix_fmt",
            "gray",
        ],
        bits_per_pixel=8,
    )
    next(reader)
    return reader


async def run(
    media: Path, search_distance: int, nb_seconds_to_extract: float
) -> search_imghash_in_db.ResultRun:
    reader = build_reader_frames(media, nb_seconds_to_extract)
    gen_frame_hash = map(rawframe_to_imghash, reader)
    gen_signed_int64_hash = map(imghash_to_signed_int64, gen_frame_hash)
    return await search_imghash_in_db.run(
        map(str, gen_signed_int64_hash), search_distance
    )


async def search_media_name_into_db(conn, media_id: int) -> str:
    return await conn.fetchval(
        f"""SELECT "name"
            FROM "medias"
            WHERE medias.id = {media_id}"""
    )


async def build_results(
    media: Path, results: search_imghash_in_db.ResultRun
) -> List[BuildResult]:
    map_media_id_to_offsets_matched: Dict[int, List[PairedMatchedFrame]] = defaultdict(
        list
    )
    for record in results.records:
        for match in record.matches:
            map_media_id_to_offsets_matched[match.media_id].append(
                PairedMatchedFrame(
                    record_search_offset=record.search_offset,
                    match_frame_offset=match.frame_offset,
                )
            )

    conn: Connection = await asyncpg.connect(
        user=psqlUserName, password=psqlUserPass, database=psqlDbName, host=psqlDbIpAddr
    )

    async def _build_result(media_id, paired_matched_frames) -> BuildResult:
        media_name_found = await search_media_name_into_db(conn, media_id)
        return BuildResult(
            media_found=media.stem == media_name_found,
            media_name_search=media.stem,
            media_id_match=media_id,
            media_name_match=media_name_found,
            nb_offsets_match=len(paired_matched_frames),
            search_offsets_match={
                paired_matched_frame.record_search_offset
                for paired_matched_frame in paired_matched_frames
            },
            match_frames_offsets={
                paired_matched_frame.match_frame_offset
                for paired_matched_frame in paired_matched_frames
            },
            timer_in_seconds=results.timer.elapsed,
        )

    results = [
        await _build_result(media_id, paired_matched_frames)
        for media_id, paired_matched_frames in map_media_id_to_offsets_matched.items()
    ]
    await conn.close()
    return results


@click.command(short_help="")
@click.option(
    "--media",
    "-m",
    required=True,
    type=click_pathlib.Path(
        exists=True, readable=True, resolve_path=True, allow_dash=True
    ),
    help="Media to recognize",
)
@click.option(
    "--search_distance", "-d", default=1, type=int, help="Search distance to use"
)
@click.option(
    "--nb_seconds_to_extract",
    "-s",
    default=1.00,
    type=float,
    help="Nb seconds to use for cutting the media and searching",
)
def recognize_media(media: Path, search_distance: int, nb_seconds_to_extract: float):
    loop = asyncio.get_event_loop()
    results_from_search_imghash_in_db: search_imghash_in_db.ResultRun = loop.run_until_complete(
        run(media, search_distance, nb_seconds_to_extract)
    )
    results: List[BuildResult] = loop.run_until_complete(
        build_results(media, results_from_search_imghash_in_db)
    )
    # https://github.com/pandas-dev/pandas/issues/21910#issuecomment-405109225
    df_results = pd.DataFrame([asdict(x) for x in results])
    # https://pandas.pydata.org/pandas-docs/stable/reference/api/pandas.DataFrame.to_csv.html
    print(f"{df_results.to_csv(index=False, header=False)}")
