import datetime
from pathlib import Path
from typing import Iterator

from imageio_ffmpeg import read_frames


def build_reader_frames(media: Path, nb_seconds_to_extract: float) -> Iterator[bytes]:
    # Read a video file
    meta = next(read_frames(str(media)))
    # extract a (frame's) chunk around/in middle of the media
    # https://trac.ffmpeg.org/wiki/Seeking#Cuttingsmallsections
    ffmpeg_seek_input_cmd = [
        "-ss",
        str(datetime.timedelta(seconds=meta["duration"] // 2)),
    ]
    ffmpeg_seek_output_cmd = [
        "-frames:v",
        str(round(nb_seconds_to_extract * meta["fps"])),
    ]
    # ffmpeg_seek_output_cmd = ["-to", str(datetime.timedelta(seconds=nb_seconds_to_extract))]
    video_filters = ", ".join(
        (
            # build_iframe_selection(),
            # rescale output frame to 32x32
            "scale=width=32:height=32",
        )
    )
    reader = read_frames(
        str(media),
        input_params=[*ffmpeg_seek_input_cmd],
        output_params=[
            *ffmpeg_seek_output_cmd,
            "-vf",
            video_filters,
            "-pix_fmt",
            "gray",
        ],
        bits_per_pixel=8,
    )
    next(reader)
    return reader


def build_iframe_selection(pict_type: str = "I") -> str:
    """
    #######################################################################
    # I-FRAME
    #######################################################################
    Select I-Frame from video cut
    warning: all cut frames will be send with I-Frames repetitions
    for example, media's frames are: (* for I-FRAMES)
      Frame_0*    Frame_1     Frame_2     Frame_3*    Frame_4
    then the filtered frames will be:
      Frame_0*    Frame_0     Frame_0     Frame_3*    Frame_3
    warning 2: select I-FRAMES seems more slower (FFMPEG) ...
    hint: try to save/remove intermediate frames and evaluate the cost
    """
    return f"select=eq(pict_type\\,{pict_type})"
