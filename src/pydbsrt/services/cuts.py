import itertools
from dataclasses import dataclass
from pathlib import Path
from typing import Final, Iterator

import distance
import pybktree

from pydbsrt.services.reader_frames import gen_read_binary_img_hash_file
from pydbsrt.tools.imghash import signed_int64_to_str_binary
from pydbsrt.tools.pairwise import pairwise
from pydbsrt.tools.search_tree import Item, SearchTree, item_distance

DEFAULT_THRESHOLD_DISTANCE_FOR_CUT: Final[int] = 16


def gen_items_from_cuts(
    phash_file: Path,
    threshold_distance_for_cut: int = DEFAULT_THRESHOLD_DISTANCE_FOR_CUT,
) -> Iterator[Item]:
    """

    :param phash_file:
    :param threshold_distance_for_cut:
    :return:
    """
    it_img_hash: Iterator[int] = (img_hash for img_hash, _, _ in gen_read_binary_img_hash_file(phash_file, 1))
    # https://docs.python.org/3.8/library/itertools.html#itertools.chain
    # a paired frames correspond to a "cut"
    # keep all paired frames with a (hamming) distance greater than a threshold (for example: 16 (different bits))
    return itertools.chain(
        *(
            (Item(s0, offset_frame), Item(s1, offset_frame + 1))
            for (offset_frame, (s0, s1)) in enumerate(pairwise(it_img_hash))
            if distance.hamming(signed_int64_to_str_binary(s0), signed_int64_to_str_binary(s1))
            > threshold_distance_for_cut
        )
    )


@dataclass
class SearchTreeFromCuts(SearchTree):
    """ """

    threshold_distance_for_cut = DEFAULT_THRESHOLD_DISTANCE_FOR_CUT

    def __post_init__(self):
        self.tree = pybktree.BKTree(
            distance_func=item_distance,
            items=gen_items_from_cuts(self.binary_img_hash_file, self.threshold_distance_for_cut),
        )
