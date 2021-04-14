# -*- coding: utf-8 -*-
"""
➜ poetry run python src/pydbsrt/app_cli.py recognize-media --help
Usage: app_cli.py recognize-media [OPTIONS]

Options:
  -m, --media PATH                Media to recognize  [required]
  -d, --search_distance INTEGER   Search distance to use
  -s, --nb_seconds_to_extract FLOAT
                                  Nb seconds to use for cutting the media and
                                  searching

  --output_format [DataFrame|CSV]
  -h, --help                      Show this message and exit.

# Description
Application to launch a search from media (video) on BKTreeDB (implement on PostgreSQL database) and printout matches

# Example
```sh
find "/home/latty/NAS/tvshow/Silicon Valley/" -type f -name "*.mkv" | \
xargs -I {} python app_cli.py recognize-media -m "{}" --output_format CSV
[...]
The frame size for reading (32, 32) is different from the source frame size (1920, 1080).
True,Silicon.Valley.S05E08.Fifty-One.Percent.1080p.AMZN.WEB-DL.DD.5.1.H.265-SiGMA,Silicon.Valley.S05E08.Fifty-One.Percent.1080p.AMZN.WEB-DL.DD.5.1.H.265-SiGMA,38,33,"{0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23}","{21891, 21892, 21893, 21894, 21895, 21896, 21897, 21898, 21899, 21900, 21901, 21902, 21903, 21904, 21905, 21906, 21907, 21908, 21909, 21910,
21911, 21912, 21913, 21914}",0.31966724100129795
```

"""
import asyncio
import datetime
from collections import defaultdict
from dataclasses import asdict, dataclass
from enum import Enum
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


OUTPUT_FORMAT = Enum("output_format", "DataFrame CSV")


def build_iframe_selection(pict_type: str = "I") -> str:
    """
    #######################################################################
    # I-FRAME
    #######################################################################
    Select I-Frame from video cut
    warning: all cut frames will be send with I-Frames repetitions
    for example, media's frames are: (* for I-FRAMES)
      Frame_0*    Frame_1     Frame_2     Frame_3*    Frame_4
    then the filtered frames will be:
      Frame_0*    Frame_0     Frame_0     Frame_3*    Frame_3
    warning 2: select I-FRAMES seems more slower (FFMPEG) ...
    hint: try to save/remove intermediate frames and evaluate the cost
    """
    return f"select=eq(pict_type\\,{pict_type})"


def build_reader_frames(media: Path, nb_seconds_to_extract: float) -> Iterator[bytes]:
    # Read a video file
    meta = next(read_frames(str(media)))
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
            # rescale output frame to 32x32
            "scale=width=32:height=32",
        )
    )
    reader = read_frames(
        str(media),
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
    search_media: Path, search_distance: int, nb_seconds_to_extract: float
) -> search_imghash_in_db.ResultRun:
    reader = build_reader_frames(search_media, nb_seconds_to_extract)
    gen_frame_hash = map(rawframe_to_imghash, reader)
    gen_signed_int64_hash = map(imghash_to_signed_int64, gen_frame_hash)
    return await search_imghash_in_db.search_phash_stream(
        map(str, gen_signed_int64_hash), search_distance
    )


async def search_media_name_into_db(conn: Connection, media_id: int) -> str:
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

    # https://magicstack.github.io/asyncpg/current/api/index.html#connection-pools
    async with asyncpg.create_pool(
        user=psqlUserName,
        password=psqlUserPass,
        database=psqlDbName,
        host=psqlDbIpAddr,
        command_timeout=60,
    ) as pool:
        async with pool.acquire() as conn:

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
@click.option(
    "--output_format",
    type=click.Choice(list(map(lambda x: x.name, OUTPUT_FORMAT)), case_sensitive=False),
    default=OUTPUT_FORMAT.DataFrame.name,
)
def recognize_media(
    media: Path,
    search_distance: int,
    nb_seconds_to_extract: float,
    output_format: OUTPUT_FORMAT,
):
    loop = asyncio.get_event_loop()
    results_from_search_imghash_in_db: search_imghash_in_db.ResultRun = (
        loop.run_until_complete(run(media, search_distance, nb_seconds_to_extract))
    )
    results: List[BuildResult] = loop.run_until_complete(
        build_results(media, results_from_search_imghash_in_db)
    )
    # https://github.com/pandas-dev/pandas/issues/21910#issuecomment-405109225
    df_results = pd.DataFrame([asdict(x) for x in results])
    console.print(
        {
            # https://pandas.pydata.org/pandas-docs/stable/reference/api/pandas.DataFrame.to_csv.html
            "CSV": df_results.to_csv(index=False, header=False),
            "DataFrame": df_results,
        }.get(output_format, df_results)
    )
