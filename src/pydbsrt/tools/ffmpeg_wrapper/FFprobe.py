"""

FFprobe Wrapper

"""
import json
import os
import subprocess

from pydbsrt.tools.ffmpeg_wrapper import FFException

binary_path = None


def initialize():
    """
    Looks for FFprobe binary with OS environment
    :return:
    """
    global binary_path

    for path in os.environ["PATH"].split(os.pathsep):
        if os.path.exists(os.path.join(path, "ffprobe")):
            binary_path = os.path.join(path, "ffprobe")
    if not binary_path:
        raise FFException.BinaryNotFound("ffprobe")


def streams(media):
    """
    JSON list of streams
    :param media:
    :return:
    """
    try:
        output = run(["-i", media.geturl(), "-print_format", "json", "-show_streams"])
        return json.loads(output)
    except FFException.BinaryCallFailed:
        return None


def stream(media, index):
    """
    JSON specific stream
    :param media:
    :param index:
    :return:
    """
    global binary_path
    try:
        output = run(
            [
                "-i",
                media.geturl(),
                "-print_format",
                "json",
                "-show_streams",
                "-select_streams",
                str(index),
            ]
        )
        return json.loads(output)["streams"][0]
    except FFException.BinaryCallFailed:
        return None


def run(opts):
    """

    :param opts:
    :return:
    """
    global binary_path
    try:
        cmd = [binary_path, "-v", "error"] + opts
        stdout = subprocess.check_output(cmd, stderr=None)
        return stdout.decode("utf-8")
    except subprocess.CalledProcessError:
        raise FFException.BinaryCallFailed
