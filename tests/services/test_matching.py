import random
from pathlib import Path

import pytest

from pydbsrt.services.matching import search_phash_stream_in_db
from pydbsrt.tools.imghash import gen_signed_int64_hash, imghash_to_signed_int64, signed_int64_to_imghash


@pytest.mark.asyncio
async def test_search_phash_stream(conn, aio_insert_phash_into_db, phash_from_media: Path):
    binary_img_hash_file = phash_from_media
    media_id, nb_frames_inserted = aio_insert_phash_into_db

    it_phashes = map(str, gen_signed_int64_hash(binary_img_hash_file.open("rb")))
    # search all frames in order with 0 searching distance (<=> exact match)
    search_results = await search_phash_stream_in_db(it_phashes, search_distance=0)
    # in all match records, we can found a match with:
    # - frame's offset equal to record's offset
    # - and media's id match equal to media's id inserted into database
    assert [
        # first match result (i.e minimal result frame offset founded)
        next(
            match.frame_offset
            for match in record.matches
            if match.frame_offset == record_offset and match.media_id == media_id
        )
        for record_offset, record in enumerate(search_results.records)
    ] == list(range(nb_frames_inserted))


@pytest.mark.asyncio
async def test_search_phash_stream_with_one_bit_revert(conn, aio_insert_phash_into_db, phash_from_media: Path):
    binary_img_hash_file = phash_from_media

    # get the first phash frame
    int64_first_phash = next(gen_signed_int64_hash(binary_img_hash_file.open("rb")))
    # revert one (random) bit of the bits array
    imghash_first_phash = signed_int64_to_imghash(int64_first_phash)
    rand_row, rand_col = random.sample(range(0, 8), 2)
    imghash_first_phash.hash[rand_row, rand_col] = not imghash_first_phash.hash[rand_row, rand_col]
    int64_first_phash = imghash_to_signed_int64(imghash_first_phash)
    # search this new phash (with one reverse bit) with search distance to zero (exact matching)
    search_results = await search_phash_stream_in_db(map(str, (int64_first_phash,)), search_distance=0)
    search_record = search_results.records[0]
    # => 0 matching found
    assert len(list(filter(lambda record: record.frame_offset == 0, search_record.matches))) == 0
    # same research with search distance equal to 1 (fuzzy search)
    search_results = await search_phash_stream_in_db(map(str, (int64_first_phash,)), search_distance=1)
    search_record = search_results.records[0]
    # => non 0 matching and one of these matches has frame_offset attribute to 0 (<=> first frame)
    assert len(list(filter(lambda record: record.frame_offset == 0, search_record.matches))) == 1
