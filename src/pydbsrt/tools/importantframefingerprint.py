"""
"""
from dataclasses import dataclass, field
from typing import Iterator

from pydbsrt.tools.imghash import imghash_count_nonzero, imghash_distance
from pydbsrt.tools.videofingerprint import VideoFingerprint


@dataclass
class ImportantFrameFingerprints:
    """
"""

    vfp: VideoFingerprint
    # THRESHOLDS
    threshold_distance: int = 8
    threshold_nonzero: int = 0

    vfp_iterator: Iterator = field(init=False)

    def __post_init__(self):
        self.vfp_iterator = iter(self.vfp)

    def __iter__(self):
        self.vfp_iterator = iter(self.vfp)
        self.last_vfp = None
        self.id_frame = 0
        return self

    def __next__(self):
        try:
            self.cur_vfp = next(self.vfp_iterator)
            self.id_frame += 1

            while self.last_vfp is None:
                if imghash_count_nonzero(self.cur_vfp) >= self.threshold_nonzero:
                    self.last_vfp = self.cur_vfp
                    return self.cur_vfp, self.id_frame
                # next frame
                self.cur_vfp = next(self.vfp_iterator)
                self.id_frame += 1

            while True:
                # tests on thresholds
                thresholds_pass_for_exit = True
                thresholds_pass_for_exit &= (
                    imghash_distance(self.last_vfp, self.cur_vfp)
                    >= self.threshold_distance
                )
                thresholds_pass_for_exit &= (
                    imghash_count_nonzero(self.cur_vfp) >= self.threshold_nonzero
                )
                if thresholds_pass_for_exit:
                    break
                # next frame
                self.cur_vfp = next(self.vfp_iterator)
                self.id_frame += 1

            self.last_vfp = self.cur_vfp
            return self.cur_vfp, self.id_frame
        except (StopIteration, Exception):
            raise StopIteration
