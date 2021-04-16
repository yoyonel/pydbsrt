#!/usr/bin/env python -*- coding: utf-8 -*-
import logging
import click

from pydbsrt.applications.export_imghash_from_media import export_imghash_from_media
from pydbsrt.services.export_imghash_from_subtitles_and_media import (
    export_imghash_from_subtitles_and_media,
)

# https://pypi.org/project/click-pathlib/
from pydbsrt.services.extract_subtitles_from_medias import extract_subtitles_from_medias
from pydbsrt.services.import_imghash_into_db import import_images_hashes_into_db
from pydbsrt.applications.recognize_media import recognize_media
from pydbsrt.services.search_imghash_in_db import search_imghash_in_db
from pydbsrt.services.show_imghash_from_subtitles_and_media import (
    show_imghash_from_subtitles_and_media,
)


@click.group(context_settings={"help_option_names": ["-h", "--help"]})
@click.option("--log-level", default="WARN", help="set logging level")
def entry_point(log_level):
    logging.getLogger(__name__).setLevel(getattr(logging, log_level.upper()))


entry_point.add_command(export_imghash_from_media)
entry_point.add_command(import_images_hashes_into_db)
entry_point.add_command(search_imghash_in_db)
entry_point.add_command(recognize_media)
entry_point.add_command(export_imghash_from_subtitles_and_media)
entry_point.add_command(show_imghash_from_subtitles_and_media)
entry_point.add_command(extract_subtitles_from_medias)

if __name__ == "__main__":
    try:
        entry_point()
    except KeyboardInterrupt:
        pass
