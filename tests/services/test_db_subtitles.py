import pytest

from pydbsrt.services.db_subtitles import import_subtitles_into_db


@pytest.mark.asyncio
async def test_import_subtitles_into_db(aio_insert_phash_into_db, phash_from_media, resource_video_path, tmp_path):
    resource_video_name = "big_buck_bunny_trailer_480p"
    p_subtitles = resource_video_path(f"{resource_video_name}.en.srt")
    binary_img_hash_file = phash_from_media

    # First insertion into DB
    res = await import_subtitles_into_db(p_subtitles, binary_img_hash_file)
    assert res.nb_row_inserted > 0

    # Second insertion (already exists, returns early)
    res_second = await import_subtitles_into_db(p_subtitles, binary_img_hash_file)
    assert res_second.nb_row_inserted == 0

    # Missing media branch
    fake_phash = tmp_path / "fake.phash"
    fake_phash.write_bytes(b"dummy")
    res_missing = await import_subtitles_into_db(p_subtitles, fake_phash, drop_before_inserting=True)
    assert res_missing.nb_row_inserted == 0
