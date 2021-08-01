# https://chrisdown.name/2016/09/04/cleaning-up-muxing-extracting-subtitles-using-ffmpeg-srt-tools.html
from pathlib import Path
from typing import Optional

from rich.console import Console
from rich.progress import BarColumn, DownloadColumn, Progress, TextColumn, TransferSpeedColumn
from rich.theme import Theme

from pydbsrt.tools.encoded_subtitles import ExceptionSubtitles, export_encoded_subtitles

# https://pypi.org/project/click-pathlib/
from pydbsrt.tools.rich_colums import TimeElapsedOverRemainingColumn

# TODO: put this configuration (theme, console, progress bar) in `tools`
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


def extract_from_medias(
    root_dir_for_finding_medias: Path,
    glob_for_finding_medias: str,
    root_dir_export_subtitles: Optional[Path],
    default_relative_path_export_for_subtitles: str,
):
    if root_dir_export_subtitles is None:
        root_dir_export_subtitles = root_dir_for_finding_medias / default_relative_path_export_for_subtitles
        root_dir_export_subtitles.mkdir(parents=True, exist_ok=True)
    console.print(f"ℹ️ {root_dir_export_subtitles=}", style="info")

    task_id = progress.add_task(
        "Retrieve subtitles streams from medias",
        filename=f"{str(root_dir_for_finding_medias)}{glob_for_finding_medias}",
        start=True,
    )
    with progress:
        for media_path in filter(Path.is_file, root_dir_for_finding_medias.glob(glob_for_finding_medias)):
            progress.update(task_id, advance=1, refresh=False)
            console.print(
                f"ℹ️ Extract encoded subtitles streams from {media_path.stem=} ...",
                style="info",
            )
            try:
                subtitles_exported_path = export_encoded_subtitles(
                    media_path,
                    root_dir_export_subtitles_path=root_dir_export_subtitles,
                    filter_srt_export_fp=lambda srt_export_fp: not srt_export_fp[1].exists(),
                )
            except ExceptionSubtitles:
                # https://superuser.com/questions/1485242/understanding-ffmpeg-error-extracting-subtitles
                # TODO: Analyze more errors/exceptions output and get back a more clearer error message
                console.print(
                    f":cross_mark: Can't extract subtitles (streams) from {media_path.name}",
                    style="danger",
                )
                continue
            if not subtitles_exported_path:
                console.print("⏭️ All subtitles already extracted ! [SKIP]", style="warning")
                continue
            console.print(
                f"ℹ️ {len(subtitles_exported_path)} subtitles exported from {media_path.stem}",
                style="info",
            )
