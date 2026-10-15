from typing import Final

import pytest

from pydbsrt.models.searching import BuildSearchResult
from pydbsrt.services import export_imghash_from_media
from pydbsrt.services.db_frames import import_binary_img_hash_to_db_async
from pydbsrt.services.matching import search_phash_stream_in_db
from pydbsrt.services.search_in_db import build_search_media_results, search_media_in_db
from pydbsrt.tools.imghash import gen_signed_int64_hash


@pytest.mark.asyncio
async def test_search_media_in_db(conn, big_buck_bunny_trailer, tmpdir):
    p_media = big_buck_bunny_trailer
    output_file_path = tmpdir.mkdir("phash") / f"{p_media.stem}.phash"
    output_file_exported = export_imghash_from_media(p_media, output_file_path)

    binary_img_hash_file = output_file_exported
    media_id, _nb_frames_inserted = await import_binary_img_hash_to_db_async(binary_img_hash_file)

    search_distance = 0
    nb_seconds_to_extract = 3.00
    video_framerate: Final = 25.0
    results_from_search_imghash_in_db = await search_media_in_db(
        p_media, search_distance, nb_seconds_to_extract, seek_to_middle=False
    )

    assert [
        [
            match.frame_offset
            for match in record.matches
            if (match.frame_offset == record_offset and match.media_id == media_id)
        ][0]
        for record_offset, record in enumerate(results_from_search_imghash_in_db.records)
    ] == list(range(int(nb_seconds_to_extract * video_framerate)))


@pytest.mark.asyncio
async def test_build_search_media_results(conn, big_buck_bunny_trailer, resource_phash_path, resource_video_path):
    media_name = big_buck_bunny_trailer.stem
    binary_img_hash_file = resource_phash_path(f"{media_name}.phash")

    # import images hashes file into (test) database
    media_id, nb_frames_inserted = await import_binary_img_hash_to_db_async(binary_img_hash_file)

    # load images hashes into memory
    it_phashes = map(str, gen_signed_int64_hash(binary_img_hash_file.open("rb")))

    # search all frames in order with 0 searching distance (<=> exact match)
    results_from_search_imghash_in_db = await search_phash_stream_in_db(it_phashes, search_distance=0)

    # build results match
    build_search_results = await build_search_media_results(big_buck_bunny_trailer, results_from_search_imghash_in_db)

    # compare
    assert build_search_results == [
        BuildSearchResult(
            media_found=True,
            media_name_search=media_name,
            media_name_match=media_name,
            media_id_match=media_id,
            nb_offsets_match=7367,
            search_offsets_match=set(range(nb_frames_inserted)),
            match_frames_offsets=set(range(nb_frames_inserted)),
            timer_in_seconds=build_search_results[0].timer_in_seconds,
            confidence=1.0,
        )
    ]
