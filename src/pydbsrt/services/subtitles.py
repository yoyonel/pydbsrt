# https://chrisdown.name/2016/09/04/cleaning-up-muxing-extracting-subtitles-using-ffmpeg-srt-tools.html
from pathlib import Path
from typing import Optional

from pydbsrt.tools.encoded_subtitles import ExceptionSubtitles, export_encoded_subtitles
from pydbsrt.tools.rich_helpers import print_error, print_info, print_warning, progress


def extract_subtitles_from_medias(
    root_dir_for_finding_medias: Path,
    glob_for_finding_medias: str,
    root_dir_export_subtitles: Optional[Path],
    default_relative_path_export_for_subtitles: Path = Path("SRT"),
) -> None:
    """
    Using `ffprobe` and `ffmpeg` trying to extract subtitles channels from medias/videos.
    This service can work on batch mode.

    Args:
        root_dir_for_finding_medias (Path): Root directory for finding/searching medias/videos
        glob_for_finding_medias (str): Glob pattern to search medias
        root_dir_export_subtitles (Optional[Path]): Root directory for exporting subtitles (from medias)
        default_relative_path_export_for_subtitles (Path): default relative (to root directory provided to find media)
         (sub)directory used if no root directory export is provided. Defaults to Path("SRT").
    """
    if root_dir_export_subtitles is None:
        root_dir_export_subtitles = root_dir_for_finding_medias / default_relative_path_export_for_subtitles
        root_dir_export_subtitles.mkdir(parents=True, exist_ok=True)
        print_warning("No root directory for export subtitles specified")
    print_info(f"{root_dir_export_subtitles=}")

    task_id = progress.add_task(
        "Retrieve subtitles streams from medias",
        filename=f"{str(root_dir_for_finding_medias)}{glob_for_finding_medias}",
        start=True,
    )
    with progress:
        for media_path in filter(Path.is_file, root_dir_for_finding_medias.glob(glob_for_finding_medias)):
            progress.update(task_id, advance=1, refresh=False)
            print_info(f"Extract encoded subtitles streams from {media_path.stem=} ...")
            try:
                subtitles_exported_path = export_encoded_subtitles(
                    media_path,
                    root_dir_export_subtitles_path=root_dir_export_subtitles,
                    cb_filter_subtitles_to_export=lambda srt_export_fp: not srt_export_fp[1].exists(),
                )
            except ExceptionSubtitles:
                # https://superuser.com/questions/1485242/understanding-ffmpeg-error-extracting-subtitles
                # TODO: Analyze more errors/exceptions output and get back a more clearer error message
                print_error(f"Can't extract subtitles (streams) from {media_path.name}")
                continue
            if not subtitles_exported_path:
                print_warning("All subtitles already extracted ! [SKIP]")
                continue
            print_info(f"{len(subtitles_exported_path)} subtitles exported from {media_path.stem}")
