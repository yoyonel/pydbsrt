"""

"""
import attr
import imagehash
import imageio
import numpy as np
from PIL import Image
#
from pydbsrt.tools.videoreader import VideoReader


@attr.s()
class VideoFingerprint:
    """

    """
    vreader = attr.ib(type=VideoReader)
    #
    func_for_hash = attr.ib(default=lambda f: imagehash.phash(f))

    frame_reader = attr.ib(init=False)

    def __attrs_post_init__(self):
        self.frame_reader = iter(self.vreader.reader)

    def __iter__(self):
        self.frame_reader = iter(self.vreader.reader)
        return self

    def __next__(self):
        try:
            cur_frame = next(self.frame_reader)
            try:
                frame = cur_frame
                img = Image.fromarray(np.uint8(frame))
                return self.func_for_hash(img)
            except imageio.core.format.CannotReadFrameError as e:
                raise StopIteration
        except StopIteration:
            raise StopIteration
