#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""
"""
import argparse
import logging
import subprocess
import re
import sys
import json

from pydbsrt.tools.ffmpeg_wrapper import FFException
from pydbsrt.tools.ffmpeg_wrapper import FFmpeg
from pydbsrt.tools.ffmpeg_wrapper import FFmpegFilter
from pydbsrt.tools.ffmpeg_wrapper import FFprobe
from pydbsrt.tools.ffmpeg_wrapper import FFMedia

logger = logging.getLogger('ffmpegtools.cut')


def set_select_expression(
        frame_start,
        frame_stop,
):
    """

    Args:
        frame_start (str or int):
        frame_stop (str or int):

    Returns:
        select_expression (str):

    Examples:
        >>> set_select_expression('BEGIN', 'END')
        'gte(n\\\,0)'
        >>> set_select_expression('BEGIN', 500)
        >>> set_select_expression(50, 500)
        'gte(n\\\,50)*lte(n\\\,500)'
        >>> set_select_expression(200, 50)
        'gte(n\\\,200)*lte(n\\\,50)'

        # Tests TODO:
        # - Specify BEGIN
        # - Specify END
        # - Specify both (BEGIN, STOP)
        # - BEGIN not in valid range ([0, n[, n: (max) number of video frames)
        # - STOP not in valid range
    """
    select_expression = ""

    if frame_start == 'BEGIN':
        frame_start = '0'
    try:
        i_frame_start = int(frame_start)
    except ValueError as e:
        logger.error("Error when trying to cast [{}] to integer.".format(frame_start))
        raise e
    else:
        if int(frame_start) < 0:
            raise ValueError('Invalid start frame [{0}].'.format(frame_start))
        select_expression += 'gte(n\,{0})'.format(i_frame_start)

    if frame_stop != 'END':
        try:
            i_frame_stop = int(frame_stop)
        except ValueError as e:
            logger.error("Error when trying to cast [{}] to integer.".format(frame_stop))
            raise e
        else:
            if i_frame_stop < 0:
                raise ValueError('Invalid stop frame [{0}].'.format(frame_stop))
            select_expression += '*lte(n\,{0})'.format(i_frame_stop)

    return select_expression


def set_aselect_expression(
        frame_start,
        frame_stop,
        input_file
):
    """

    Args:
        frame_start (str or int):
        frame_stop (str or int):
        input_file (str):

    Returns:
        select_expression (str):

    Examples:
        >>> set_aselect_expression('BEGIN', 'END', '')
        'gte(t\\\,0)'

        # Tests TODO:
        # - Specify BEGIN
        # - Specify END
        # - Specify both (BEGIN, STOP)
        # - BEGIN not in valid range ([0, n[, n: (max) number of video frames)
        # - STOP not in valid range
        # - input_file doesn't exist
    """
    aselect_expression = ""

    if frame_start == 'BEGIN':
        aselect_expression += 'gte(t\,0)'
    else:
        try:
            i_frame_start = int(frame_start)
        except ValueError as e:
            logger.error("Error when trying to cast [{}] to integer.".format(frame_start))
            raise e
        else:
            if int(frame_start) < 0:
                raise ValueError('Invalid start frame [{0}].'.format(frame_start))
            aselect_expression += 'gte(t\,{0})'.format(calculate_frame_pts_time(i_frame_start, input_file))

    if frame_stop != 'END':
        try:
            i_frame_stop = int(frame_stop)
        except ValueError as e:
            logger.error("Error when trying to cast [{}] to integer.".format(frame_stop))
            raise e
        else:
            if i_frame_stop < 0:
                raise ValueError('Invalid stop frame [{0}].'.format(frame_stop))
            aselect_expression += '*lte(t\,{0})'.format(calculate_frame_pts_time(i_frame_stop, input_file))

    return aselect_expression


def calculate_frame_pts_time(
        n,
        media_fp
):
    """

        Args:
            n (int): Frame indice
            media_fp (str): Media file path

        Returns:
            pts_time(float): Presentation Time Stamp in seconds
    """
    regex = re.compile('(\d+)/(\d+)')

    try:
        m = FFMedia.FFMedia(media_fp)
        frame_rate_entry = m.stream_entry("v:0", "r_frame_rate")
        match = regex.match(frame_rate_entry)
        pts_time = None
        if match:
            frame_rate = int(match.group(1)) / int(match.group(2))
            pts_time = n / frame_rate
        return pts_time
    except FFException.InvalidMedia as e:
        logger.error("Invalid media [{}].".format(e.media_path))
    except subprocess.CalledProcessError as e:
        logger.error('FFprobe failed.')
        raise e
    except json.JSONDecodeError as e:
        logger.error('Unable to decode json from FFprobe output.')
        raise e
    except ValueError as e:
        logger.error('FFprobe \'r_frame_rate\' entry does not seem valid')
        raise e


def cut(
        input_file,
        output_file,
        frame_start="BEGIN",
        frame_stop="END",
        decoder="",
        encoder="",
):
    """

    Args:
        input_file(str):
        output_file(str):
        decoder(str):
        encoder(str):
        frame_start(str):
        frame_stop(str):

    Returns:
        return_code(int): FFmpeg return code
    """

    try:
        select_expression = set_select_expression(frame_start, frame_stop)
        aselect_expression = set_aselect_expression(frame_start, frame_stop, input_file)

        logger.info("Video Select Expression [{0}]".format(select_expression))
        logger.info("Audio Select Expression [{0}]".format(aselect_expression))

        ffmpeg = FFmpeg.FFmpeg()
        ffmpeg.add_input_file(input_file)
        ffmpeg.set_output_file(output_file)
        ffmpeg.set_video_decoder(decoder)
        ffmpeg.set_video_encoder(encoder)

        f_select = FFmpegFilter.FFmpegFilter('select')
        f_select.set_option('expr', select_expression)
        ffmpeg.add_video_filter(f_select)

        f_setpts = FFmpegFilter.FFmpegFilter('setpts')
        f_setpts.set_option('expr', 'PTS-STARTPTS')
        ffmpeg.add_video_filter(f_setpts)

        f_aselect = FFmpegFilter.FFmpegFilter('aselect')
        f_aselect.set_option('expr', aselect_expression)
        ffmpeg.add_audio_filter(f_aselect)

        f_asetpts = FFmpegFilter.FFmpegFilter('asetpts')
        f_asetpts.set_option('expr', 'N/SR/TB')
        ffmpeg.add_audio_filter(f_asetpts)

        with ffmpeg.build().run() as proc:
            out, err = proc.communicate()

        if proc.returncode != 0:
            logger.error(err.decode("utf-8"))
        return proc.returncode
    except subprocess.CalledProcessError:
        logger.error('FFmpeg failed.')


def process(args):
    """

    Args:
        args : **argparse.Namespace**

    Returns:

    """
    try:
        FFmpeg.initialize()
        FFprobe.initialize()

    except FFException.BinaryNotFound as e:
        logger.error("Binary not found: [{0}].".format(e.binary_name))
    else:
        logger.info("ffmpeg binary found at [{0}].".format(FFmpeg.binary_path))
        logger.info("ffprobe binary found at [{0}].".format(FFprobe.binary_path))

        return cut(
            args.input_file,
            args.output_file,
            args.frame_start,
            args.frame_stop,
            args.decoder,
            args.encoder,
        )


def build_parser(parser=None, **argparse_options):
    """
    Args:
        parser (argparse.ArgumentParser):
        **argparse_options (dict):
    Returns:
    """
    if parser is None:
        parser = argparse.ArgumentParser(
            **argparse_options,
            description="""Search files recursively starting at 'base_directory'
            and count the differents codecs found"""
        )

    # config file
    parser.add_argument("-i", '--input-file', dest="input_file",
                        required=True,
                        help="", metavar="FILE")
    parser.add_argument("-o", '--output-file', dest="output_file",
                        required=True,
                        help="", metavar="FILE")

    parser.add_argument("-fstart", '--frame-start', dest="frame_start",
                        type=str,
                        default='BEGIN',
                        required=False,
                        help="")

    parser.add_argument("-fstop", '--frame-stop', dest="frame_stop",
                        type=str,
                        default='END',
                        required=False,
                        help="")

    parser.add_argument("-dec", '--decoder', dest="decoder",
                        type=str,
                        default="",
                        required=False,
                        help="")

    parser.add_argument("-enc", '--encoder', dest="encoder",
                        type=str,
                        default="",
                        required=False,
                        help="")
    #
    parser.add_argument("-v", "--verbose", action="store_true", default=False,
                        help="increase output verbosity")
    # return parsing
    return parser


def parse_arguments():
    """
    Returns:
        parser (any): **argparse.Namespace**
    """
    # return parsing
    return build_parser().parse_args()


def main():
    args = parse_arguments()
    sys.exit(process(args))


if __name__ == '__main__':
    main()
