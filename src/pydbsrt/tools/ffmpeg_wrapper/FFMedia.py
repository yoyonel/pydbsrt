import subprocess
from urllib.parse import urlparse

from pydbsrt.tools.ffmpeg_wrapper import FFException, FFprobe


class FFMedia(object):
    """
    Checks Data Validity for FFprobe
    """

    def __init__(self, url):
        """
        Checks Data Validity
        :param url:
        """
        self.parsed_url = urlparse(url)
        self.streams = dict()

        try:
            subprocess.check_call(
                [
                    FFprobe.binary_path,
                    "-loglevel",
                    "quiet",
                    "-i",
                    self.parsed_url.geturl(),
                ],
                stdin=None,
                stdout=None,
                stderr=None,
            )
        except subprocess.CalledProcessError:
            raise FFException.InvalidMedia(self.parsed_url.geturl())

    def stream_entry(self, stream_index, stream_entry):
        """
        :param stream_index:
        :param stream_entry:
        :return:
        """
        try:
            if stream_index not in self.streams.keys():
                self.streams[stream_index] = FFprobe.stream(self, stream_index)
            return self.streams[stream_index][stream_entry]
        except IndexError:
            raise FFException.StreamIndexOutOfBound
        except KeyError:
            raise FFException.StreamEntryNotFound

    def geturl(self):
        """
        Full Path of Media
        :return:
        """
        return self.parsed_url.geturl()
