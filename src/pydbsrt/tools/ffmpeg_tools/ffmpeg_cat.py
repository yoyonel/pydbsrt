#!/usr/bin/env python3

import os
import sys
import argparse
import tempfile
import json
import logging
import subprocess

from pydbsrt.tools.ffmpeg_wrapper import FFException
from pydbsrt.tools.ffmpeg_wrapper import FFprobe
from pydbsrt.tools.ffmpeg_wrapper import FFMedia
from pydbsrt.tools.ffmpeg_wrapper import FFmpeg

logger = logging.getLogger('ffmpegtools.cat')


def get_video_scale(media_fp):
    """

    Args:
        media_fp:

    Returns:

    """
    try:
        media = FFMedia.FFMedia(media_fp)
        width = media.stream_entry("v:0", "width")
        height = media.stream_entry("v:0", "height")
        return '{}x{}'.format(width, height)
    except subprocess.CalledProcessError as e:
        logger.error('FFprobe failed.')
        raise e
    except json.JSONDecodeError as e:
        logger.error('Unable to decode json from FFprobe output.')
        raise e
    except ValueError as e:
        logger.error('FFprobe \'r_frame_rate\' entry does not seem valid')
        raise e


def get_audio_codec(media_fp):
    """

    Args:
        media_fp:

    Returns:

    """
    try:
        media = FFMedia.FFMedia(media_fp)
        codec = media.stream_entry("a:0", "codec_name")
        return codec
    except subprocess.CalledProcessError as e:
        logger.error('FFprobe failed.')
        raise e
    except json.JSONDecodeError as e:
        logger.error('Unable to decode json from FFprobe output.')
        raise e
    except ValueError as e:
        logger.error('FFprobe \'r_frame_rate\' entry does not seem valid')
        raise e


def get_video_codec(media_fp):
    """

    Args:
        media_fp:

    Returns:

    """
    try:
        media = FFMedia.FFMedia(media_fp)
        codec = media.stream_entry("v:0", "codec_name")
        return codec
    except subprocess.CalledProcessError as e:
        logger.error('FFprobe failed.')
        raise e
    except json.JSONDecodeError as e:
        logger.error('Unable to decode json from FFprobe output.')
        raise e
    except ValueError as e:
        logger.error('FFprobe \'r_frame_rate\' entry does not seem valid')
        raise e


def cat(out_media_fp, l_in_media_fp):
    """
    Args:
        out_media_fp(str): Output Media File Path
        l_in_media_fp(list): List of Media File Path
    Returns:
        return_code(int):
    """
    ref_vcodec = get_video_codec(l_in_media_fp[0])
    ref_acodec = get_audio_codec(l_in_media_fp[0])
    ref_vscale = get_video_scale(l_in_media_fp[0])
    for f in l_in_media_fp:
        if ref_vcodec != get_video_codec(f):
            logger.error('Video Codecs are different.')
            return -1
        if ref_acodec != get_audio_codec(f):
            logger.error('Audio Codecs are different.')
            return -1
        if ref_vscale != get_video_scale(f):
            logger.error('Video Scales are different.')
            return -1

    ffmpeg = FFmpeg.FFmpeg()
    ffmpeg.set_output_file(out_media_fp)
    ffmpeg.set_input_format('concat')
    ffmpeg.set_video_encoder('copy')
    ffmpeg.set_audio_encoder('copy')
    ffmpeg.set_safe(0)
    try:
        fpath = tempfile.mkstemp()[1]
        with open(fpath, 'w') as fp:
            for media_fp in l_in_media_fp:
                fp.write('file \'{}\'\n'.format(os.path.abspath(media_fp)))

        ffmpeg.add_input_file(fpath)

        with ffmpeg.build().run() as proc:
            out, err = proc.communicate()
            logger.error(err.decode("utf-8"))

        os.remove(fpath)
        return proc.returncode
    except subprocess.CalledProcessError:
        logger.error('FFmpeg failed.')


def process(args):
    """

    Args:
        args:

    Returns:

    """
    if len(args.l_in_media_fp) < 2:
        logger.error('Too few input media.')
        return -1
    try:
        FFmpeg.initialize()
        FFprobe.initialize()
    except FFException.BinaryNotFound as e:
        logger.error("Binary not found: [{0}].".format(e.binary_name))

    for media_fp in args.l_in_media_fp:
        if not os.path.exists(media_fp):
            raise FileNotFoundError
    return cat(args.out_media_fp, args.l_in_media_fp)


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

    parser.add_argument('-i', '--input',
                        nargs='+',
                        required=True,
                        dest='l_in_media_fp'
                        )

    parser.add_argument('-o', '--output',
                        required=True,
                        dest='out_media_fp'
                        )
    return parser


def parse_arguments():
    """
    Returns:
        argparse.Namespace:
    """
    # return parsing
    return build_parser().parse_args()


def main():
    args = parse_arguments()
    sys.exit(process(args))


if __name__ == '__main__':
    main()
