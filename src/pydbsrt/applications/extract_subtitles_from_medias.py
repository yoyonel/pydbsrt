"""
➜ poetry run python src/pydbsrt/app_cli.py extract-subtitles-from-medias --help
Usage: app_cli.py extract-subtitles-from-medias [OPTIONS]
                                                ROOT_DIR_FOR_FINDING_MEDIAS
                                                GLOB_FOR_FINDING_MEDIAS

Options:
  --version                       Show the version and exit.
  --root-dir-export-subtitles PATH
  --default-relative-path-export-for-subtitles TEXT
  -h, --help                      Show this message and exit.

# Description
Outil permettant d'extraire les streams de sous-titres des medias/vidéos MKV
L'outil est concu pour fonctionner en batch, en fournissant un répertoire racine et une pattern (glob) de recherche

# Examples
## Bach mode
`extract-subtitles-from-medias "/home/latty/NAS/tvshow" "**/*.mkv"`
par défaut enregistre les sous-titres dans le répertoire "/home/latty/NAS/tvshow/SRT",
au format: `<file stem du media>_<lang>_<index>.srt`
"""
import sys

# https://chrisdown.name/2016/09/04/cleaning-up-muxing-extracting-subtitles-using-ffmpeg-srt-tools.html
from pathlib import Path
from signal import SIGHUP, SIGINT, SIGTERM, SIGUSR1, SIGUSR2, signal
from typing import Optional

import click

# https://pypi.org/project/click-pathlib/
import click_pathlib

from pydbsrt.services import extract_from_medias

__version__ = "0.0.1"

ALL_HANDLING_SIGNALS = [
    SIGINT,
    SIGHUP,
    SIGTERM,
    SIGUSR1,
    SIGUSR2,
]


def sh_shutdown(_sig, _):
    print("SHUTDOWN ...")

    # deactivate signals (handling) during shutdown
    def _signal_handler(_sig, _trace):
        pass

    for sig in ALL_HANDLING_SIGNALS:
        signal(sig, _signal_handler)

    sys.exit()


@click.command(short_help="Tool for extracting subtitles from medias/videos (using ffprobe + ffmpeg)")
@click.version_option(version=__version__)
@click.argument(
    "root-dir-for-finding-medias",
    type=click_pathlib.Path(exists=True, readable=True, resolve_path=True, allow_dash=False),
)
@click.argument(
    "glob-for-finding-medias",
    type=str,
)
@click.option(
    "--root-dir-export-subtitles",
    required=False,
    type=click_pathlib.Path(exists=True, writable=True, resolve_path=True),
)
@click.option(
    "--default-relative-path-export-for-subtitles",
    required=False,
    default="SRT",
    type=str,
)
def extract_subtitles_from_medias(
    root_dir_for_finding_medias: Path,
    glob_for_finding_medias: str,
    root_dir_export_subtitles: Optional[Path],
    default_relative_path_export_for_subtitles: str,
):
    for sig in ALL_HANDLING_SIGNALS:
        signal(sig, sh_shutdown)

    extract_from_medias(
        root_dir_for_finding_medias,
        glob_for_finding_medias,
        root_dir_export_subtitles,
        default_relative_path_export_for_subtitles,
    )
