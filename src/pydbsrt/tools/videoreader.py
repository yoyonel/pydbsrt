"""
"""
from dataclasses import dataclass, field
import imageio
from imageio.core.format import Format
from pathlib import Path


@dataclass
class VideoReader:
    media_path: Path
    #
    reader: Format.Reader = field(init=False)
    metadatas: dict = field(init=False)

    def __post_init__(self):
        self.reader = imageio.get_reader(self.media_path, 'ffmpeg')
        self.metadatas = self.reader.get_meta_data()
