""" """

import logging
from dataclasses import dataclass, field
from typing import Any, Callable, Iterator, cast

import imagehash
import imageio
import numpy as np
from imagehash import ImageHash
from PIL import Image
from PIL.Image import Image as PILImage

#
from pydbsrt.tools.videoreader import VideoReader

logger = logging.getLogger(__name__)

CannotReadFrameError = getattr(getattr(imageio, "core", None), "CannotReadFrameError", (IndexError, RuntimeError))


@dataclass
class VideoFingerprint:
    """"""

    @staticmethod
    def _default_hash(f: PILImage) -> ImageHash:
        return imagehash.phash(f)

    video_reader: VideoReader
    #
    func_for_hash: Callable[[PILImage], ImageHash] = _default_hash

    frame_reader: Iterator[Any] = field(init=False)

    def __post_init__(self):
        self.frame_reader = iter(cast(Any, self.video_reader.reader))

    def __iter__(self) -> Iterator[ImageHash]:
        self.frame_reader = iter(cast(Any, self.video_reader.reader))
        return self

    def __next__(self) -> ImageHash:
        try:
            cur_frame = next(self.frame_reader)
            try:
                frame = cur_frame
                img = Image.fromarray(np.asarray(frame, dtype=np.uint8))
                return self.func_for_hash(img)
            except CannotReadFrameError as e:
                # TODO: refactor this !
                logger.error(repr(e))
                raise StopIteration
            except Exception as e:
                logger.error(repr(e))
                raise RuntimeError(e)
        except StopIteration:
            raise
