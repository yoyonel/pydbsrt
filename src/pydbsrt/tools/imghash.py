"""

"""
# import binascii
from bitstring import BitArray
import distance
from imagehash import ImageHash
import numpy as np


def imghash_to_64bits(imghash: ImageHash) -> str:
    """

    :param imghash:
    :return:

    >>> imghash_to_64bits(ImageHash(np.array([\
        np.array([ True,  True, False,  True, False,  True, False,  True]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False])])))
    '1101010100000000000000000000000000000000000000000000000000000000'
    """
    # binascii.b2a_hex(imghash.hash.flatten())[:64] -> b'0101000100010001000000000000000000000000000000000000000000000000'
    # binascii.a2b_hex(str(imghash)) -> b'\xd5\x00\x00\x00\x00\x00\x00\x00'
    return BitArray(f'0x{str(imghash)}').bin  # -> '1101010100000000000000000000000000000000000000000000000000000000'


def imghash_distance(
        imghash0: ImageHash,
        imghash1: ImageHash,
        distance_func=distance.hamming
) -> int:
    """

    :param imghash0:
    :param imghash1:
    :return:

    >>> ih0 = ImageHash(np.array([\
        np.array([ True,  True, False,  True, False,  True, False,  True]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False])]))
    >>> ih1 = ImageHash(np.array([\
        np.array([ True,  True, False,  True, False,  True, False,  True]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, True,  False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False])]))
    >>> imghash_distance(ih0, ih1)
    1
    """
    return distance_func(imghash_to_64bits(imghash0), imghash_to_64bits(imghash1))


def imghash_count_nonzero(imghash: ImageHash) -> int:
    """

    :param imghash:
    :return:

    >>> imghash_count_nonzero(ImageHash(np.array([\
        np.array([ True,  True, False,  True, False,  True, False,  True]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False])])))
    5
    """
    return np.count_nonzero(imghash.hash.flatten())