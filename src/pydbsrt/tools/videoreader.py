"""
"""
from dataclasses import dataclass, field
from pathlib import Path

import imageio
from imageio.core.format import Format


@dataclass
class VideoReader:
    media_path: Path
    #
    reader: Format.Reader = field(init=False)
    metadatas: dict = field(init=False)

    def __post_init__(self):
        self.reader = imageio.get_reader(self.media_path, "ffmpeg")
        self.metadatas = self.reader.get_meta_data()
