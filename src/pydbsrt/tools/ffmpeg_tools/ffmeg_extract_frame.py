"""
"""
import imagehash
import logging
import numpy as np
from PIL import Image
import subprocess
import time
from typing import Generator

# from holimetrix.protos.crawler import Frame_pb2
from pydbsrt.tools.ffmpeg_tools.ffmpeg_cut import set_select_expression
from pydbsrt.tools.ffmpeg_wrapper import FFmpeg
from pydbsrt.tools.ffmpeg_wrapper import FFmpegFilter


def ffmpeg_frame_generator(
        input: str,
        start_frame: int = None,
        stop_frame: int = None,
        frame_width: int = 32,
        frame_height: int = 32,
) -> Generator[bytes, None, None]:
    """

    :param input: path or url of the media to read frames from
    :type input: str

    :param start_frame: Index of the starting frame. Use None to start at the beginning of the media
    :type start_frame: int

    :param stop_frame: Index of the ending frame. Use None to stop at the ending of the media
    :type stop_frame: int

    :param frame_width: Width of the result frame. Default to 32
    :type frame_width: int

    :param frame_height: Height of the result frame. Default to 32
    :type frame_height: int

    :return:
    :rtype: collections.Iterable[bytes]
    """

    frame_size = frame_width * frame_height

    ffmpeg = FFmpeg.FFmpeg()
    ffmpeg.add_input_file(input)
    ffmpeg.set_output_file('-')
    ffmpeg.set_output_format('image2pipe')
    ffmpeg.set_video_encoder('rawvideo')

    f_scale = FFmpegFilter.FFmpegFilter('scale')
    f_scale.set_option('width', frame_width)
    f_scale.set_option('height', frame_height)
    ffmpeg.add_video_filter(f_scale)

    # Frame selections, based on
    # https://github.com/Holimetrix/hmx-ffmpeg-tools/blob/master/src/holimetrix/ffmpeg_tools/ffmpeg_cut.py#L208-L222

    if start_frame is None:
        start_frame = 'BEGIN'

    if stop_frame is None:
        stop_frame = 'END'

    select_expression = set_select_expression(start_frame, stop_frame)

    f_select = FFmpegFilter.FFmpegFilter('select')
    f_select.set_option('expr', select_expression)
    ffmpeg.add_video_filter(f_select)

    f_setpts = FFmpegFilter.FFmpegFilter('setpts')
    f_setpts.set_option('expr', 'PTS-STARTPTS')
    ffmpeg.add_video_filter(f_setpts)

    ffmpeg.set_pixel_format('gray')
    try:
        nb_frames = 0

        proc = ffmpeg.build().run()

        frame_data = proc.stdout.read(frame_size)
        while len(frame_data) > 0:
            nb_frames += 1
            yield frame_data
            frame_data = proc.stdout.read(frame_size)

        proc.wait()
        proc.stdout.close()

        return nb_frames
    except subprocess.CalledProcessError:
        logging.error('FFmpeg failed.')
        return -1


def rawframe_to_imghash(
        raw_frame: bytes,
        frame_width: int = 32,
        frame_height: int = 32,
) -> imagehash.ImageHash:
    return imagehash.phash(
        Image.fromarray(
            # https://docs.scipy.org/doc/numpy-1.14.0/reference/generated/numpy.frombuffer.html
            np.frombuffer(raw_frame,
                          dtype=np.uint8,
                          count=frame_width * frame_height).reshape(frame_width, frame_height)
        )
    )


def ffmpeg_imghash_generator(
        input: str,
        start_frame: int = None,
        stop_frame: int = None,
        frame_width: int = 32,
        frame_height: int = 32,
) -> Generator[imagehash.ImageHash, None, None]:
    for raw_frame in ffmpeg_frame_generator(input, start_frame, stop_frame, frame_width, frame_height):
        yield rawframe_to_imghash(raw_frame)


if __name__ == '__main__':
    from pathlib import Path

    # root_path = Path('data/')
    # media_path = root_path.joinpath('big_buck_bunny_trailer_480p.webm')
    root_path = Path("/home/latty/Vidéos/Mission Impossible Rogue Nation (2015) [1080p]/")
    media_path = root_path.joinpath("Mission.Impossible.Rogue.Nation.2015.1080p.BluRay.x264.YIFY.[YTS.AG].mp4")

    gen_imghash = ffmpeg_imghash_generator(str(media_path))

    # https://pythonhow.com/measure-execution-time-python-code/
    t0 = time.time()
    speed = None
    sum_speed = 0
    nb_mesures_for_timing = 10

    for i, imghash in enumerate(gen_imghash):
        if not i % 25:
            t1 = time.time()
            sum_speed += 1.0 / (t1 - t0)
            print(f"New frames: {i} - {i / 25}s - {imghash}", end='')
            if not i % (25 * nb_mesures_for_timing):
                speed = sum_speed / float(nb_mesures_for_timing)
                sum_speed = 0
                print(f"- x{speed}")
            else:
                print()
            t0 = time.time()
