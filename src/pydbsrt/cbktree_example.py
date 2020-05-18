"""
https://github.com/gpip/cBKTree#example
"""
from cbktree import BkHammingTree, explicitSignCast

DATA = {
    # Format: id -> bitstring
    1: '1011010010010110110111111000001000001000100011110001010110111011',
    2: '1011010010010110110111111000001000000001100011110001010110111011',
    3: '1101011110100100001011001101001110010011100010011101001000110101',
}

SEARCH_DIST = 2  # 2 out of 64 bits

int_bits = lambda b: explicitSignCast(int(b, 2))

tree = BkHammingTree()
descriptor = {}
for node_id, bits in DATA.items():
    ib = int_bits(bits)
    descriptor[node_id] = ib
    tree.insert(ib, node_id)

# Find near matches for each node that was inserted.
for node_id, ib in descriptor.items():
    res = tree.getWithinDistance(ib, SEARCH_DIST)
    print("{}: {}".format(node_id, res))

# Find near matches for items that were not inserted.
dupes = lambda x: tree.getWithinDistance(int_bits(x), SEARCH_DIST)
new = '1101011110100100001011001101001110010011100010011101001000110101'
assert dupes(new) == {3}

ones = '1' * 64
assert dupes(ones) == set()

zeroes = '0' * 64
assert dupes(zeroes) == set()
