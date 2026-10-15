import binascii
import hashlib
from typing import List

import pytest

from pydbsrt.tools.aio_filehash import hashfile


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "sample_size,sample_threshold,size,expected_hash",
    (
        (16384, 131072, 0, "00000000000000000000000000000000"),
        (16384, 131072, 1, "01659e2ec0f3c75bf39e43a41adb5d4f"),
        (16384, 131072, 127, "7f47671cc79d4374404b807249f3166e"),
        (16384, 131072, 128, "800183e5dbea2e5199ef7c8ea963a463"),
        (16384, 131072, 4095, "ff1f770d90d3773949d89880efa17e60"),
        (16384, 131072, 4096, "802048c26d66de432dbfc71afca6705d"),
        (16384, 131072, 131072, "8080085a3d3af2cb4b3a957811cdf370"),
        (16384, 131073, 131072, "808008282d3f3b53e1fd132cc51fcc1d"),
        (16384, 131072, 500000, "a0c21e44a0ba3bddee802a9d1c5332ca"),
        (50, 131072, 300000, "e0a712edd8815c606344aed13c44adcf"),
    ),
)
async def test_aio_hashfile(tmpdir, sample_size, sample_threshold, size, expected_hash):
    def _generate_bin_hash(_size: int) -> bytes:
        chunks: List[bytes] = []
        hasher = hashlib.md5()  # nosec
        while 16 * len(chunks) < _size:
            hasher.update(b"A")
            chunks.append(hasher.digest())

        return b"".join(chunks)[:_size]

    test_data_path = tmpdir / ".test_data"
    with test_data_path.open("wb") as f:
        f.write(_generate_bin_hash(size))
    result = await hashfile(test_data_path, sample_threshold=sample_threshold, sample_size=sample_size)
    assert binascii.hexlify(result) == expected_hash.encode()
