#!/usr/bin/env python -*- coding: utf-8 -*-
import sys

import click
from loguru import logger

from pydbsrt.applications.export_imghash_from_media import export_imghash_from_media
from pydbsrt.applications.export_imghash_from_subtitles_and_media import export_imghash_from_subtitles_and_media

# https://pypi.org/project/click-pathlib/
from pydbsrt.applications.extract_subtitles_from_medias import extract_subtitles_from_medias
from pydbsrt.applications.import_imghash_into_db import import_images_hashes_into_db
from pydbsrt.applications.import_subtitles_into_db import import_subtitles_into_db
from pydbsrt.applications.recognize_media import recognize_media
from pydbsrt.applications.search_imghash_in_db import search_imghash_in_db
from pydbsrt.applications.show_imghash_from_subtitles_and_media import show_imghash_from_subtitles_and_media
from pydbsrt.applications.show_imghash_from_subtitles_and_media_in_db import show_imghash_from_subtitles_and_media_in_db
from pydbsrt.applications.wip_generate_subtitles import generate_subtitles


@click.group(context_settings={"help_option_names": ["-h", "--help"]})
# @click.option("--log-level", default="WARN", help="set logging level")
# def entry_point(log_level):
def entry_point():
    # logging.getLogger(__name__).setLevel(getattr(logging, log_level.upper()))
    logger.add(sys.stdout, colorize=True, backtrace=True, diagnose=True)


for command_name in (
    export_imghash_from_media,
    import_images_hashes_into_db,
    search_imghash_in_db,
    recognize_media,
    export_imghash_from_subtitles_and_media,
    show_imghash_from_subtitles_and_media,
    extract_subtitles_from_medias,
    show_imghash_from_subtitles_and_media_in_db,
    import_subtitles_into_db,
    generate_subtitles,
):
    entry_point.add_command(command_name)

if __name__ == "__main__":
    try:
        entry_point()
    except KeyboardInterrupt:
        pass
