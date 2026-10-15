from pathlib import Path
from typing import Final

import pytest

from pydbsrt.services.matching import search_phash_stream_in_db
from pydbsrt.tools.imghash import gen_signed_int64_hash


@pytest.mark.asyncio
async def test_search_imghash_in_db(conn, aio_insert_phash_into_db, phash_from_media: Path):
    search_distance: Final = 0

    media_id_inserted, nb_phash_from_media_inserted = aio_insert_phash_into_db

    with phash_from_media.open("rb") as fo:
        phash_stream = map(str, gen_signed_int64_hash(fo))
        search_results = await search_phash_stream_in_db(phash_stream, search_distance)
    assert len(search_results.records) == nb_phash_from_media_inserted
    # for each search result, we try to find in match result the offset (frame media) of the input search
    # in all match records, we can found a match with:
    # - frame's offset equal to record's offset
    # - and media's id match equal to media's id inserted into database
    assert [
        # first match result (i.e minimal result frame offset founded)
        next(
            match.frame_offset
            for match in record.matches
            if match.frame_offset == record_offset and match.media_id == media_id_inserted
        )
        for record_offset, record in enumerate(search_results.records)
    ] == list(range(nb_phash_from_media_inserted))
