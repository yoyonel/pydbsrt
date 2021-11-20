# -*- coding: utf-8 -*-
"""
➜ poetry run python src/pydbsrt/app_cli.py retarget-subtitles --help
Usage: app_cli.py retarget-subtitles [OPTIONS]

Options:
  --ref_subtitles PATH   Path to subtitles file  [required]
  --ref_media PATH       Path to media  [required]
  --target_media PATH    Path to media  [required]
  --phash_dir DIRECTORY  Path to PHash
  -h, --help             Show this message and exit.

# Description

# Example
➜ poetry run python src/pydbsrt/app_cli.py \
    retarget-subtitles \
        --ref_subtitles "/media/nas/volume1/video/Dallas Buyers Club (2013) [1080p]/Dallas.Buyers.Club.2013.1080p.BluRay.x264.YIFY.fr.srt"
        --ref_media "/media/nas/volume1/video/Dallas Buyers Club (2013) [1080p]/Dallas.Buyers.Club.2013.1080p.BluRay.x264.YIFY.mp4"
        --target_media "/media/nas/volume1/video/Dallas Buyers Club (2013)[1080p-RARBG][en srt, pas fr]/Dallas.Buyers.Club.2013.1080p.BluRay.x265-RARBG.mp4"
        --phash_dir "/media/nas/volume1/video/__PHASH__"

ref_phash_file_hash='88935221d94ee19b085ca771c53b65be'
target_phash_file_hash='f09252dfd8e1cada8486c937dcd6dbad'
target_st=_retarget_subtitles_with_bktree.<locals>._SearchTree(binary_img_hash_file=PosixPath('.'), media_id=0, tree=<BKTree using item_distance with 18 top-level nodes>)
Build BKTrees for target => took 1.7696 seconds
Nb paired frames used: 2549 frames ~ 106.20833333333333 seconds
Search differences offsets between reference and target => took 0.6674 seconds
Find differences offsets for synchronize subtitles (reference -> target) => took 2.4386 seconds
target_frame_rate=23.98 fps
Shift subtitles seconds = 118 frames ~ 4.9208 seconds
Exported subtitles for target in : '/tmp/Dallas.Buyers.Club.2013.1080p.BluRay.x265-RARBG.mp4.23.98fps.srt'
Shift subtitles (from reference) and save it (for target) => took 0.0565 seconds
"""
import shutil
from pathlib import Path
from tempfile import gettempdir

import click

# https://pypi.org/project/click-pathlib/
import click_pathlib
from loguru import logger
from rich.console import Console

from pydbsrt import services
from pydbsrt.services import retarget_subtitles_async
from pydbsrt.tools.coro import coroclick

console = Console()


@click.command(
    short_help="Tool for retarget subtitles from reference to target medias (same 'movie' but different releases)"
)
@click.option(
    "--ref_subtitles",
    required=True,
    type=click_pathlib.Path(exists=True, readable=True, resolve_path=True, allow_dash=False),
    help="Path to subtitles file",
)
@click.option(
    "--ref_media",
    required=True,
    type=click_pathlib.Path(exists=True, readable=True, resolve_path=True, allow_dash=False),
    help="Path to media",
)
@click.option(
    "--target_media",
    required=True,
    type=click_pathlib.Path(exists=True, readable=True, resolve_path=True, allow_dash=False),
    help="Path to media",
)
@click.option(
    "--phash_dir",
    default=gettempdir(),
    type=click_pathlib.Path(
        exists=True, readable=True, resolve_path=True, allow_dash=False, dir_okay=True, file_okay=False
    ),
    help="Path to PHash",
)
@coroclick
@logger.catch
async def retarget_subtitles(ref_subtitles, ref_media, target_media, phash_dir):
    def _find_or_build_phash_from_media(media_file: Path) -> Path:
        try:
            # search existing phash file
            phash_file = next(phash_dir.glob(f"*{media_file.stem}*.phash"))
        except StopIteration:
            console.print(f"Building PHash for reference: '{media_file}")
            phash_file = services.export_imghash_from_media(media_file)
            # https://docs.python.org/3/library/shutil.html#shutil.move
            phash_file = shutil.move(phash_file, phash_dir.joinpath(phash_file.name))
            console.print(f"PHash for reference export to: '{phash_file}'")
        return phash_file

    ref_phash_file = _find_or_build_phash_from_media(ref_media)
    target_phash_file = _find_or_build_phash_from_media(target_media)

    await retarget_subtitles_async(ref_subtitles, ref_phash_file, target_phash_file)
