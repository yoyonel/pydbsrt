import contextlib
import datetime
from pathlib import Path
from typing import Dict, Iterator, Optional, Tuple

from imageio_ffmpeg import read_frames
from rich.progress import Progress

from pydbsrt.tools.imghash import bytes_to_signed_int64
from pydbsrt.tools.silence_capture_stdout_stderr import suppress_stdout_stderr

SIZE_IMG_HASH = 8


def build_iframe_selection(pict_type: str = "I") -> str:
    """
    #######################################################################
    # I-FRAME
    #######################################################################
    Select I-Frame from video cut
    warning: all cut frames will be sent with I-Frames repetitions
    for example, media's frames are: (* for I-FRAMES)
      Frame_0*    Frame_1     Frame_2     Frame_3*    Frame_4
    then the filtered frames will be:
      Frame_0*    Frame_0     Frame_0     Frame_3*    Frame_3
    warning 2: select I-FRAMES seems more slower (FFMPEG) ...
    hint: try to save/remove intermediate frames and evaluate the cost
    """
    return f"select=eq(pict_type\\,{pict_type})"


def build_reader_frames(
    media: Path,
    nb_seconds_to_extract: float = 0,
    seek_to_middle: bool = False,
    ffmpeg_reduce_verbosity: bool = True,
    ignore_stderr_with_ffmpeg: bool = False,
) -> Tuple[Iterator[bytes], Dict]:
    meta = {}
    ffmpeg_seek_input_cmd = []
    ffmpeg_seek_output_cmd = []

    if ffmpeg_reduce_verbosity or seek_to_middle:
        meta = next(read_frames(str(media)))

    # extract a (frame's) chunk around/in middle of the media
    # https://trac.ffmpeg.org/wiki/Seeking#Cuttingsmallsections
    if ffmpeg_reduce_verbosity:
        ffmpeg_seek_input_cmd += "-hide_banner -nostats -nostdin".split(" ")
    if seek_to_middle:
        ffmpeg_seek_input_cmd += f"-ss {str(datetime.timedelta(seconds=meta['duration'] // 2))}".split(" ")

    if nb_seconds_to_extract:
        ffmpeg_seek_output_cmd += [
            "-frames:v",
            str(round(nb_seconds_to_extract * meta["fps"])),
        ]
    # ffmpeg_seek_output_cmd = ["-to", str(datetime.timedelta(seconds=nb_seconds_to_extract))]

    # rescale output frame to 32x32
    video_filters = "scale=width=32:height=32"
    # video_filters += f", {build_iframe_selection()}"

    reader = read_frames(
        str(media),
        input_params=[*ffmpeg_seek_input_cmd],
        output_params=[
            *ffmpeg_seek_output_cmd,
            *f"-vf {video_filters}".split(" "),
            *"-pix_fmt gray".split(" "),
        ],
        bits_per_pixel=8,
    )

    if ignore_stderr_with_ffmpeg:
        # remove: The frame size for reading (32, 32) is different from the source frame size (_,__).
        cm = suppress_stdout_stderr()
    else:
        cm = contextlib.nullcontext()
    with cm:
        meta = next(reader)

    return reader, meta


def gen_read_binary_img_hash_file(
    binary_img_hash_file: Path,
    media_id: int,
    progress: Optional[Progress] = None,
) -> Iterator[Tuple[int, int, int]]:
    with binary_img_hash_file.open("rb") as fo:
        if progress:
            task_id = progress.add_task(
                "insert img_hash into db",
                filename=binary_img_hash_file.name,
                start=True,
            )
            # https://docs.python.org/3/library/pathlib.html#pathlib.Path.stat
            progress.update(task_id, total=binary_img_hash_file.stat().st_size // SIZE_IMG_HASH)
        with progress or contextlib.nullcontext():
            # TODO: maybe trying to read more bytes (packed chunk) to optimize (need to profile/evaluate)
            ba_img_hex = fo.read(SIZE_IMG_HASH)
            offset_frame = 0
            while ba_img_hex:
                yield bytes_to_signed_int64(ba_img_hex), offset_frame, media_id
                ba_img_hex = fo.read(SIZE_IMG_HASH)
                offset_frame += 1
                if progress:
                    progress.update(task_id, advance=1, refresh=False)
