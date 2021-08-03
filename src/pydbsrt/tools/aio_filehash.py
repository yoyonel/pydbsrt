import binascii
import os

import aiofiles
import mmh3
import varint

SAMPLE_THRESHOLD = 128 * 1024
SAMPLE_SIZE = 16 * 1024


async def _aio_get_data_for_hash(aio_fo, sample_threshold: int = SAMPLE_THRESHOLD, sample_size: int = SAMPLE_SIZE):
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


async def aio_hashfileobject(
    aio_fo,
    sample_threshold: int = SAMPLE_THRESHOLD,
    sample_size: int = SAMPLE_SIZE,
    hexdigest: bool = False,
):
    size, data = await _aio_get_data_for_hash(aio_fo, sample_threshold, sample_size)

    hash_tmp = mmh3.hash_bytes(data)
    hash_ = hash_tmp[7::-1] + hash_tmp[16:7:-1]
    enc_size = varint.encode(size)
    # https://black.readthedocs.io/en/stable/the_black_code_style/current_style.html#slices
    digest = enc_size + hash_[len(enc_size) :]

    return binascii.hexlify(digest).decode() if hexdigest else digest


async def aio_hashfile(
    filename,
    sample_threshold=SAMPLE_THRESHOLD,
    sample_size=SAMPLE_SIZE,
    hexdigest=False,
):
    async with aiofiles.open(filename, mode="rb") as aio_fo:
        return await aio_hashfileobject(aio_fo, sample_threshold, sample_size, hexdigest)
