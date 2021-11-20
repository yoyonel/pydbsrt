import collections
from dataclasses import dataclass, field
from pathlib import Path
from typing import List, Tuple

import pybktree

from pydbsrt.services.reader_frames import gen_read_binary_img_hash_file

Item = collections.namedtuple('Item', 'bits id')

MatchedItem = Tuple[int, Item]


def item_distance(x, y):
    return pybktree.hamming_distance(x.bits, y.bits)


@dataclass
class SearchTree:
    binary_img_hash_file: Path
    media_id: int = 0

    tree: pybktree.BKTree = field(init=False)

    def __post_init__(self):
        self.tree = pybktree.BKTree(
            item_distance,
            (
                Item(p_hash, frame_offset)
                for p_hash, frame_offset, _ in gen_read_binary_img_hash_file(self.binary_img_hash_file, self.media_id)
            ),
        )

    def update(self, binary_img_hash_file):
        """
        Build a search (bK)Tree from
        """
        tree = self.tree
        for p_hash, frame_offset, _ in gen_read_binary_img_hash_file(binary_img_hash_file, 0):
            tree.add(Item(p_hash, frame_offset))

    def find(self, item, distance) -> List[MatchedItem]:
        return self.tree.find(item, distance)

    def __repr__(self):
        return repr(self.tree)
