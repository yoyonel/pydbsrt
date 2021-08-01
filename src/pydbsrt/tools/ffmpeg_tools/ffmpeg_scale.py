#!/usr/bin/python3

import argparse
import logging
import os
import subprocess
import sys

from pydbsrt.tools.ffmpeg_wrapper import FFException, FFmpeg, FFmpegFilter, FFprobe

logger = logging.getLogger("ffmpegtools.scale")


def scale(
    input_file,
    output_file,
    height,
    width,
    decoder="",
    encoder="",
    aspect_ratio="disable",
):

    ffmpeg = FFmpeg.FFmpeg()

    ffmpeg.add_input_file(os.path.abspath(input_file))
    ffmpeg.set_output_file(os.path.abspath(output_file))

    ffmpeg.set_video_decoder(decoder)
    ffmpeg.set_video_encoder(encoder)

    f_scale = FFmpegFilter.FFmpegFilter("scale")
    f_scale.set_option("height", height)
    f_scale.set_option("width", width)
    f_scale.set_option("force_original_aspect_ratio", aspect_ratio)

    ffmpeg.add_video_filter(f_scale)
    try:
        with ffmpeg.build().run() as proc:
            out, err = proc.communicate()
            logger.error(err.decode("utf-8"))
        return proc.returncode
    except subprocess.CalledProcessError:
        logger.error("FFmpeg failed.")


def process(args):
    try:
        FFmpeg.initialize()
        FFprobe.initialize()
    except FFException.BinaryNotFound as e:
        logger.error("Binary not found: [{0}].".format(e.binary_name))
    else:
        logger.info("ffmpeg binary found at [{0}].".format(FFmpeg.binary_path))
        logger.info("ffprobe binary found at [{0}].".format(FFprobe.binary_path))

    return scale(
        input_file=args.input_file,
        output_file=args.output_file,
        height=args.output_height,
        width=args.output_width,
        decoder=args.decoder,
        encoder=args.encoder,
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
    parser.add_argument("-i", "--input-file", dest="input_file", required=True, help="", metavar="FILE")
    parser.add_argument(
        "-o",
        "--output-file",
        dest="output_file",
        required=True,
        help="",
        metavar="FILE",
    )

    parser.add_argument(
        "-H",
        "--height",
        dest="output_height",
        type=str,
        default="0",
        required=False,
        help="",
    )

    parser.add_argument(
        "-W",
        "--width",
        dest="output_width",
        type=str,
        default="0",
        required=False,
        help="",
    )

    parser.add_argument(
        "-ar",
        "--aspect-ratio",
        dest="aspect_ratio",
        type=str,
        default="disable",
        required=False,
        help="",
    )

    parser.add_argument(
        "-dec",
        "--decoder",
        dest="decoder",
        type=str,
        default="",
        required=False,
        help="",
    )

    parser.add_argument(
        "-enc",
        "--encoder",
        dest="encoder",
        type=str,
        default="",
        required=False,
        help="",
    )
    #
    parser.add_argument(
        "-v",
        "--verbose",
        action="store_true",
        default=False,
        help="increase output verbosity",
    )
    # return parsing
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


if __name__ == "__main__":
    main()
