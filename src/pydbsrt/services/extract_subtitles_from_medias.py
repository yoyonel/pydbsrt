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

# Exemples
## Bach mode
`extract-subtitles-from-medias "/home/latty/NAS/tvshow" "**/*.mkv"`
par défaut enregistre les sous-titres dans le répertoire "/home/latty/NAS/tvshow/SRT",
au format: `<file stem du media>_<lang>_<index>.srt`
"""
# https://chrisdown.name/2016/09/04/cleaning-up-muxing-extracting-subtitles-using-ffmpeg-srt-tools.html
from collections import defaultdict
from functools import partial
from pathlib import Path
from typing import Optional, List, Tuple

import click

# https://pypi.org/project/click-pathlib/
import click_pathlib
import sh
from rich.console import Console
from rich.progress import (
    BarColumn,
    DownloadColumn,
    TextColumn,
    TransferSpeedColumn,
)
from rich.theme import Theme

from pydbsrt.tools.rich_colums import TimeElapsedOverRemainingColumn

__version__ = "0.0.1"

from rich.progress import Progress

from yaspin import yaspin

custom_theme = Theme({"info": "dim cyan", "warning": "magenta", "danger": "bold red"})
console = Console(theme=custom_theme)

progress = Progress(
    TextColumn("[bold blue]{task.fields[filename]}", justify="right"),
    BarColumn(bar_width=None),
    "[progress.percentage]{task.percentage:>3.1f}%",
    "•",
    DownloadColumn(),
    "•",
    TransferSpeedColumn(),
    "•",
    TimeElapsedOverRemainingColumn(),
    console=console,
)


@click.command(
    short_help="Tool for extracting subtitles from medias/videos (using ffprobe + ffmpeg)"
)
@click.version_option(version=__version__)
@click.argument(
    "root-dir-for-finding-medias",
    type=click_pathlib.Path(
        exists=True, readable=True, resolve_path=True, allow_dash=False
    ),
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
    if root_dir_export_subtitles is None:
        root_dir_export_subtitles = (
            root_dir_for_finding_medias / default_relative_path_export_for_subtitles
        )
        root_dir_export_subtitles.mkdir(parents=True, exist_ok=True)
    console.print(f"ℹ️ {root_dir_export_subtitles=}", style="info")

    map_media_srt_streams = defaultdict(list)

    def extract_stream_subtitles_informations(
        _media_path: Path, stream_subtitles: str
    ) -> None:
        stream_subtitles_fields = stream_subtitles.strip().split(",")
        try:
            idx, lang = stream_subtitles_fields[:2]
        except ValueError:
            progress.console.print(
                f"{_media_path.name=}: can't extract (idx, lang) from {stream_subtitles=}",
                style="warning",
            )
            idx = stream_subtitles_fields[0]
            lang = "unk"
        map_media_srt_streams[_media_path.name].append((idx, lang))

    # https://docs.python.org/3/library/pathlib.html#pathlib.Path.glob
    # https://pymotw.com/2/glob/
    # https://pubs.opengroup.org/onlinepubs/000095399/utilities/xcu_chap02.html#tag_02_13
    with yaspin(
        text=f"Searching all medias at '{str(root_dir_for_finding_medias)}' with pattern='{glob_for_finding_medias}'"
    ) as spinner:
        medias_paths = list(
            filter(
                Path.is_file, root_dir_for_finding_medias.glob(glob_for_finding_medias)
            )
        )
    spinner.ok("✅ ")

    def build_srt_export_filepath(idx, lang) -> Path:
        return root_dir_export_subtitles / f"{media_path.name}_{lang}_{idx}.srt"

    task_id = progress.add_task(
        "Retrieve subtitles streams from medias",
        filename=f"{str(root_dir_for_finding_medias)}{glob_for_finding_medias}",
        start=True,
    )
    progress.update(task_id, total=len(medias_paths))
    with progress:
        for media_path in medias_paths:
            try:
                # "Extract all subtitles from a movie using ffprobe & ffmpeg"
                # https://gist.github.com/kowalcj0/ae0bdc43018e2718fb75290079b8839a
                # https://ffmpeg.org/ffprobe.html
                sh.ffprobe(
                    *(
                        "-loglevel error "
                        "-select_streams s -show_entries stream=index:stream_tags=language:stream_tags=title "
                        "-of csv=p=0".split(" "),
                        str(media_path),
                    ),
                    _out=partial(extract_stream_subtitles_informations, media_path),
                    _bg=False,
                )
            # TODO: Need better exceptions handling
            except (sh.ErrorReturnCode_1, Exception):
                console.print_exception()
            progress.update(task_id, advance=1, refresh=False)
            streams_srt = map_media_srt_streams[media_path.name]
            if streams_srt:
                console.print(f"{media_path.name} - {len(streams_srt)=}", style="info")
                list_srt_export_filepath: List[Tuple[int, Path]] = list(
                    filter(
                        lambda tup: not tup[1].exists(),
                        [
                            (idx, build_srt_export_filepath(idx, lang))
                            for idx, lang in streams_srt
                        ],
                    )
                )
                if not list_srt_export_filepath:
                    console.print(
                        "⏭️ All subtitles already extracted ! [SKIP]", style="warning"
                    )
                    continue
                # https://trac.ffmpeg.org/wiki/Creating%20multiple%20outputs
                ffmpeg_map_cmd_for_srt = [
                    ("-map", f"0:{idx}", srt_export_filepath)
                    for idx, srt_export_filepath in list_srt_export_filepath
                ]
                progress.console.print(
                    f"▶️ Extract {len(streams_srt)} subtitles from {media_path.name}"
                )
                try:
                    sh.ffmpeg(
                        *(
                            "-y -nostdin -hide_banner -loglevel quiet -i".split(" "),
                            str(media_path),
                            *ffmpeg_map_cmd_for_srt,
                        ),
                        _bg=False,
                    )
                # TODO: Need better exceptions handling
                except (sh.ErrorReturnCode_1, Exception):
                    console.print_exception()
                    console.print(
                        f":cross_mark: Can't extract subtitles (streams) from {media_path.name}",
                        style="danger",
                    )
                    raise

    console.print(
        f"ℹ️ map_media_srt_streams: #keys={len(map_media_srt_streams)=}, "
        f"#values={sum(len(str_streams) for str_streams in map_media_srt_streams.values())}",
        style="info",
    )
