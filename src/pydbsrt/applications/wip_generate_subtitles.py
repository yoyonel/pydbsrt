# -*- coding: utf-8 -*-
"""
➜ poetry run python src/pydbsrt/app_cli.py generate-subtitles-from-db \

# Description

"""
import asyncio
from pathlib import Path
from typing import List

import click

# https://pypi.org/project/click-pathlib/
import click_pathlib
from rich.console import Console

from pydbsrt.services.extended_subtitles import read_extended_subtitles
from pydbsrt.tools.subfingerprint import subriptime_to_frame
from pydbsrt.tools.subreader import SubReader

console = Console()

SIZE_IMG_HASH = 8


async def run(
    src_subtitles: Path,
    src_binary_extended_subtitles: Path,
    target_subtitles_lang: str = "FR",
):
    # spinner = yaspin(
    #     Spinners.dots2, f"", timer=True
    # )
    src_p_hash_subtitles: List[int] = list(
        read_extended_subtitles(src_binary_extended_subtitles)
    )

    # console.print(f"len(p_hash_subtitles) = {len(src_p_hash_subtitles)}\n{src_p_hash_subtitles[:25]}")
    cur_src_p_hash_subtitles = 0
    for sub_rip in SubReader(src_subtitles):
        sub_rip_frame_start, sub_rip_frame_end = (
            subriptime_to_frame(sub_rip.start),
            subriptime_to_frame(sub_rip.end),
        )
        sub_rip_nb_frames = sub_rip_frame_end - sub_rip_frame_start
        next_cur_src_p_hash_subtitles = cur_src_p_hash_subtitles + sub_rip_nb_frames
        if next_cur_src_p_hash_subtitles > len(src_p_hash_subtitles):
            break
        sub_rip_p_hash_start, sub_rip_p_hash_end = (
            src_p_hash_subtitles[cur_src_p_hash_subtitles],
            src_p_hash_subtitles[next_cur_src_p_hash_subtitles - 1],
        )
        cur_src_p_hash_subtitles += sub_rip_nb_frames
        console.print(
            f"{sub_rip.index} - sub_rip_p_hash_start, sub_rip_p_hash_end = ({sub_rip_p_hash_start}, {sub_rip_p_hash_end})\n"
            f"\tsub_rip_frame_start, sub_rip_frame_end = ({sub_rip_frame_start}, {sub_rip_frame_end})"
        )
    console.print(cur_src_p_hash_subtitles)


@click.command(short_help="")
@click.option(
    "--src_subtitles",
    "-s",
    required=True,
    type=click_pathlib.Path(
        exists=True, readable=True, resolve_path=True, allow_dash=False
    ),
    help="Path to source subtitles file",
)
@click.option(
    "--src_binary_extended_subtitles",
    "-b",
    required=True,
    type=click_pathlib.Path(
        exists=True, readable=True, resolve_path=True, allow_dash=False
    ),
    help="Path to source subtitles file",
)
def generate_subtitles(src_subtitles, src_binary_extended_subtitles):
    loop = asyncio.get_event_loop()
    loop.run_until_complete(run(src_subtitles, src_binary_extended_subtitles))
