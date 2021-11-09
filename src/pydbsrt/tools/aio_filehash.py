import binascii
import os
from typing import Tuple, Union

import aiofiles
import mmh3
import varint

from pydbsrt.tools.constants import SAMPLE_SIZE, SAMPLE_THRESHOLD


async def _aio_get_data_for_hash(
    aio_fo, sample_threshold: int = SAMPLE_THRESHOLD, sample_size: int = SAMPLE_SIZE
) -> Tuple[int, bytes]:
    """

    :param aio_fo:
    :param sample_threshold:
    :param sample_size:
    :return:
    """
    await aio_fo.seek(0, os.SEEK_END)
    size = await aio_fo.tell()
    await aio_fo.seek(0, os.SEEK_SET)

    if size < sample_threshold or sample_size < 1:
        data = await aio_fo.read()
    else:
        data = await aio_fo.read(sample_size)
        await aio_fo.seek(size // 2)
        data += await aio_fo.read(sample_size)
        await aio_fo.seek(-sample_size, os.SEEK_END)
        data += await aio_fo.read(sample_size)
    return size, data


async def aio_hashfile(
    filename: str,
    sample_threshold: int = SAMPLE_THRESHOLD,
    sample_size: int = SAMPLE_SIZE,
    hexdigest: bool = False,
) -> Union[str, bytes]:
    """

    :param filename:
    :param sample_threshold:
    :param sample_size:
    :param hexdigest:
    :return:
    """
    async with aiofiles.open(filename, mode="rb") as aio_fo:
        size, data = await _aio_get_data_for_hash(aio_fo, sample_threshold, sample_size)

        hash_tmp = mmh3.hash_bytes(data)
        hash_ = hash_tmp[7::-1] + hash_tmp[16:7:-1]
        enc_size = varint.encode(size)
        # https://black.readthedocs.io/en/stable/the_black_code_style/current_style.html#slices
        digest = enc_size + hash_[len(enc_size) :]

        return binascii.hexlify(digest).decode() if hexdigest else digest
