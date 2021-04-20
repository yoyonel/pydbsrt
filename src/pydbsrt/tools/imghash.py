"""
"""
import binascii

import distance
import numpy as np
from bitstring import BitArray
from imagehash import ImageHash


class ImgHashExtended(ImageHash):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.hash = self.hash.hash

    def __len__(self):
        return self.hash.size

    def __int__(self):
        ret = 0
        mask = 1 << len(self) - 1
        for bit in np.nditer(
            self.hash, order="C"
        ):  # Specify memory order, so we're (theoretically) platform agnostic
            if bit:
                ret |= mask
            mask >>= 1

        # Convert to signed representation
        VALSIZE = 64
        if ret >= 2 ** (VALSIZE - 1):
            ret -= 2 ** VALSIZE
        return ret


def imghash_to_bytes(imghash: ImageHash) -> bytes:
    """
    >>> imghash_to_bytes(ImageHash(np.array([\
        np.array([ True,  True, False,  True, False,  True, False,  True]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False])])))
    b'\\xd5\\x00\\x00\\x00\\x00\\x00\\x00\\x00'
    """
    return binascii.a2b_hex(str(imghash))


def imghash_to_bitarray(imghash: ImageHash) -> BitArray:
    return BitArray(f"0x{str(imghash)}")


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
    return BitArray(f"0x{str(imghash)}").bin


def bytes_to_signed_int64(binary_signed_int64: bytes, byteorder: str = "big") -> int:
    """
    >>> bytes_to_signed_int64(b'\\xd5\\x00\\x00\\x00\\x00\\x00\\x00\\x00')
    -3098476543630901248
    """
    # https://docs.python.org/3/library/stdtypes.html#int.from_bytes
    return int.from_bytes(binary_signed_int64, byteorder, signed=True)


def imghash_to_signed_int64(imghash: ImageHash) -> int:
    """
    >>> imghash_to_signed_int64(ImageHash(np.array([\
        np.array([ True,  True, False,  True, False,  True, False,  True]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False]), \
        np.array([False, False, False, False, False, False, False, False])])))
    -3098476543630901248
    """
    return int(ImgHashExtended(imghash))


def signed_int64_to_str_binary(imghash_signed_int64: int) -> str:
    """
    >>> signed_int64_to_str_binary(-3098476543630901248)
    '1101010100000000000000000000000000000000000000000000000000000000'
    >>> signed_int64_to_str_binary(0)
    '0000000000000000000000000000000000000000000000000000000000000000'
    """
    # TODO: benchmark these alternatives
    # https://docs.scipy.org/doc/numpy-1.15.0/reference/generated/numpy.binary_repr.html#numpy-binary-repr
    # return np.binary_repr(imghash_signed_int64 & 0xffffffffffffffff, width=64)
    # https://stackoverflow.com/a/20766900 (How to convert signed to unsigned integer in python)
    # https://stackoverflow.com/questions/339007/how-to-pad-zeroes-to-a-string
    return bin(imghash_signed_int64 & 0xFFFFFFFFFFFFFFFF)[2:].zfill(64)


def imghash_distance(
    imghash0: ImageHash, imghash1: ImageHash, distance_func=distance.hamming
) -> int:
    """

    :param imghash0:
    :param imghash1:
    :param distance_func:
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


def imghash_str_hex_to_str_binary(imghash_hex: hex) -> str:
    """

    :param imghash_hex:
    :return:


    >>> imghash_str_hex_to_str_binary("c165924de35876d9")
    '1100000101100101100100100100110111100011010110000111011011011001'

    """
    return bin(int(imghash_hex, 16))[2:]
