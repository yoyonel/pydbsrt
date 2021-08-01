import pytest

from pydbsrt.services.db_frames import import_binary_img_hash_to_db_async
from pydbsrt.services.matching import search_phash_stream
from pydbsrt.tools.imghash import gen_signed_int64_hash, imghash_to_signed_int64, signed_int64_to_imghash


@pytest.mark.asyncio
async def test_search_phash_stream(conn, resource_phash_path):
    binary_img_hash_file = resource_phash_path("big_buck_bunny_trailer_480p.phash")
    media_id, nb_frames_inserted = await import_binary_img_hash_to_db_async(binary_img_hash_file)

    it_phashes = map(str, gen_signed_int64_hash(binary_img_hash_file.open("rb")))
    # search all frames in order with 0 searching distance (<=> exact match)
    search_results = await search_phash_stream(it_phashes, search_distance=0)
    # in all match records, we can found a match with:
    # - frame's offset equal to record's offset
    # - and media's id match equal to media's id inserted into database
    assert [
        [
            match.frame_offset
            for match in record.matches
            if match.frame_offset == record_offset and match.media_id == media_id
        ][0]
        for record_offset, record in enumerate(search_results.records)
    ] == list(range(nb_frames_inserted))

    # get the first phash frame
    int64_first_phash = next(gen_signed_int64_hash(binary_img_hash_file.open("rb")))
    # revert one bit of the bits array (the middle)
    imghash_first_phash = signed_int64_to_imghash(int64_first_phash)
    imghash_first_phash.hash[4, 4] = not imghash_first_phash.hash[4, 4]
    int64_first_phash = imghash_to_signed_int64(imghash_first_phash)
    # search this new phash (with one reverse bit) with search distance to zero (exact matching)
    search_results = await search_phash_stream(map(str, (int64_first_phash,)), search_distance=0)
    search_record = search_results.records[0]
    # => 0 matching found
    assert len(list(filter(lambda record: record.frame_offset == 0, search_record.matches))) == 0
    # same research with search distance equal to 1 (fuzzy search)
    search_results = await search_phash_stream(map(str, (int64_first_phash,)), search_distance=1)
    search_record = search_results.records[0]
    # => non 0 matching and one of these matches has frame_offset attribute to 0 (<=> first frame)
    assert len(list(filter(lambda record: record.frame_offset == 0, search_record.matches))) == 1
