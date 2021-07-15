from copy import deepcopy

import numpy as np
import pytest
from imagehash import ImageHash
from pytest_lazyfixture import lazy_fixture
from tools.imghash import (
    imghash_to_bytes, imghash_to_64bits, bytes_to_signed_int64,
    imghash_to_signed_int64, signed_int64_to_str_binary, imghash_distance,
    imghash_count_nonzero, imghash_str_hex_to_str_binary, signed_int64_to_str_hex
)


@pytest.fixture
def imghash():
    return ImageHash(np.array([
        np.array([True, True, False, True, False, True, False, True]),
        np.array([False, False, False, False, False, False, False, False]),
        np.array([False, False, False, False, False, False, False, False]),
        np.array([False, False, False, False, False, False, False, False]),
        np.array([False, False, False, False, False, False, False, False]),
        np.array([False, False, False, False, False, False, False, False]),
        np.array([False, False, False, False, False, False, False, False]),
        np.array([False, False, False, False, False, False, False, False])]))


@pytest.fixture
def bin_imghash() -> bytes:
    return b'\xd5\x00\x00\x00\x00\x00\x00\x00'


@pytest.fixture
def str_imghash() -> str:
    return "11010101" \
           "00000000" \
           "00000000" \
           "00000000" \
           "00000000" \
           "00000000" \
           "00000000" \
           "00000000"


@pytest.fixture
def int64_imghash() -> int:
    return -3098476543630901248


def test_imghash_to_bytes(imghash, bin_imghash):
    result = imghash_to_bytes(imghash)
    expected_result = bin_imghash
    assert result == expected_result


def test_imghash_to_64bits(imghash, str_imghash):
    result = imghash_to_64bits(imghash)
    expected_result = str_imghash
    assert result == expected_result


def test_bytes_to_signed_int64(bin_imghash, int64_imghash):
    result = bytes_to_signed_int64(bin_imghash)
    expected_result = int64_imghash
    assert result == expected_result


def test_imghash_to_signed_int64(imghash, int64_imghash):
    result = imghash_to_signed_int64(imghash)
    expected_result = int64_imghash
    assert result == expected_result


@pytest.mark.parametrize(
    "input_int64_imghash,expected_result",
    [(lazy_fixture("int64_imghash"), lazy_fixture("str_imghash")),
     (0, "0" * 64)]
)
def test_signed_int64_to_str_binary(input_int64_imghash, expected_result):
    result = signed_int64_to_str_binary(input_int64_imghash)
    assert result == expected_result


def test_imghash_distance(imghash):
    ih0 = imghash
    ih1 = deepcopy(imghash)
    # revert one bit
    ih1.hash[0, 2] = not ih1.hash[0, 2]
    result = imghash_distance(ih0, ih1)
    expected_result = 1
    assert result == expected_result


def test_imghash_count_nonzero(imghash):
    result = imghash_count_nonzero(imghash)
    expected_result = 5
    assert result == expected_result


def test_imghash_str_hex_to_str_binary(str_imghash, int64_imghash):
    result = imghash_str_hex_to_str_binary(signed_int64_to_str_hex(int64_imghash))
    expected_result = str_imghash
    assert result == expected_result
