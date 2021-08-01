# https://chrisdown.name/2016/09/04/cleaning-up-muxing-extracting-subtitles-using-ffmpeg-srt-tools.html
from dataclasses import dataclass
from pathlib import Path
from tempfile import gettempdir
from typing import Callable, List, Tuple

# https://pypi.org/project/click-pathlib/
import sh


@dataclass(frozen=True)
class SubtitleStream:
    idx: int
    lang: str


class ExceptionSubtitles(Exception):
    """
    Base class for all exceptions defined by :mod:`websockets`.
    """

    pass


class ExceptionExtractSubtitlesStreams(ExceptionSubtitles):
    pass


class ExceptionExportEncodedSubtitles(ExceptionSubtitles):
    pass


def extract_subtitles_streams(media_path: Path) -> List[SubtitleStream]:
    srt_streams: List[SubtitleStream] = []

    def _cb_list_str_streams(stream_subtitles: str) -> None:
        stream_subtitles_fields = stream_subtitles.strip().split(",")
        try:
            idx, lang = stream_subtitles_fields[:2]
        except ValueError:
            idx = stream_subtitles_fields[0]
            lang = "unk"
        srt_streams.append(SubtitleStream(int(idx), lang))

    try:
        # "Extract all subtitles from a movie using ffprobe & ffmpeg"
        # https://gist.github.com/kowalcj0/ae0bdc43018e2718fb75290079b8839a
        # https://ffmpeg.org/ffprobe.html
        proc_sh_cmd = sh.ffprobe(
            *(
                "-loglevel error "
                # ’s’ for subtitle
                "-select_streams s "
                "-show_entries stream=index:stream_tags=language:stream_tags=title "
                "-of csv=p=0".split(" "),
                str(media_path),
            ),
            _out=_cb_list_str_streams,
            _bg=True,
        )
        proc_sh_cmd.wait()
        # TODO: Need a better exceptions handling
    except (sh.ErrorReturnCode_1, Exception) as e:
        raise ExceptionExtractSubtitlesStreams(
            "Exception occurred during shell command for extract subtitles streams !", e
        )

    return srt_streams


def export_encoded_subtitles(
    media_path: Path,
    root_dir_export_subtitles_path: Path = Path(gettempdir()),
    filter_srt_export_fp: Callable[[Tuple[int, Path]], bool] = lambda _: True,
) -> List[Path]:
    """"""

    def build_srt_export_filepath(sub_stream: SubtitleStream) -> Path:
        return root_dir_export_subtitles_path / f"{media_path.name}_{sub_stream.lang}_{sub_stream.idx}.srt"

    list_srt_export_filepath: List[Tuple[int, Path]] = list(
        filter(
            filter_srt_export_fp,
            [
                (sub_stream.idx, build_srt_export_filepath(sub_stream))
                for sub_stream in extract_subtitles_streams(media_path)
            ],
        )
    )
    if not list_srt_export_filepath:
        return []

    # https://trac.ffmpeg.org/wiki/Creating%20multiple%20outputs
    ffmpeg_map_cmd_for_srt = [
        ("-map", f"0:{idx}", srt_export_filepath) for idx, srt_export_filepath in list_srt_export_filepath
    ]

    try:
        proc_sh_cmd = sh.ffmpeg(
            *(
                # -loglevel quiet
                "-y -nostdin -hide_banner -i".split(" "),
                str(media_path),
                *ffmpeg_map_cmd_for_srt,
            ),
            _bg=True,
        )
        proc_sh_cmd.wait()
    # TODO: Need a better exceptions handling
    except (sh.ErrorReturnCode_1, Exception) as e:
        raise ExceptionExportEncodedSubtitles(
            "Exception occurred during shell command for extract subtitles streams !", e
        )

    return [srt_export_filepath for _, srt_export_filepath in list_srt_export_filepath]


if __name__ == "__main__":
    _media_path = Path(
        "/media/nas/volume1/tvshow/The.Office.US.S04.1080p.BluRay.x264-BTN[rartv]/The.Office.US.S04E03E04.Dunder.Mifflin.Infinity.1080p.BluRay.DDP5.1.x264-BTN.mkv"
    )
    if _media_path.exists():
        print(extract_subtitles_streams(_media_path))
        print(export_encoded_subtitles(_media_path))
