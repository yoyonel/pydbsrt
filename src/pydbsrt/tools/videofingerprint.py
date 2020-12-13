"""
"""
import logging
from dataclasses import dataclass, field
from typing import Iterator

import imagehash
import imageio
import numpy as np
from PIL import Image
from imagehash import ImageHash

#
from pydbsrt.tools.videoreader import VideoReader

logger = logging.getLogger(__name__)


@dataclass
class VideoFingerprint:
    """"""

    @staticmethod
    def _default_hash(f):
        return imagehash.phash(f)

    video_reader: VideoReader
    #
    func_for_hash = _default_hash

    frame_reader: Iterator = field(init=False)

    def __post_init__(self):
        self.frame_reader = iter(self.video_reader.reader)

    def __iter__(self) -> Iterator[ImageHash]:
        self.frame_reader = iter(self.video_reader.reader)
        return self

    def __next__(self) -> ImageHash:
        try:
            cur_frame = next(self.frame_reader)
            try:
                frame = cur_frame
                img = Image.fromarray(np.uint8(frame))
                return self.func_for_hash(img)
            except imageio.core.format.CannotReadFrameError as e:
                # TODO: refactor this !
                logger.error(repr(e))
                raise StopIteration
            except Exception as e:
                logger.error(repr(e))
                raise RuntimeError(e)
        except StopIteration:
            raise
