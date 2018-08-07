"""

"""
import attr
import imageio
from imageio.core.format import Format
from pathlib import Path


@attr.s()
class VideoReader:
    media_path = attr.ib(type=Path)

    reader = attr.ib(init=False, type=Format)
    metadatas = attr.ib(init=False)

    def __attrs_post_init__(self):
        self.reader = imageio.get_reader(self.media_path, 'ffmpeg')
        self.metadatas = self.reader.get_meta_data()
