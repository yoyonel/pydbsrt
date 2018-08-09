
from pydbsrt.tools.ffmpeg_wrapper import FFException


class FFStream(object):
    """
    FFmpeg Stream
    """
    def __init__(self, data):
        self.data = data

    def get(self, entry):
        try:
            return self.data[entry]
        except KeyError:
            raise FFException.StreamEntryNotFound

