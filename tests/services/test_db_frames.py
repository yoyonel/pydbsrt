import pytest
from imohash import hashfile

from pydbsrt.services.db_frames import import_binary_img_hash_to_db_async
from pydbsrt.tools.db_frames import agen_p_hash_from_media_in_db
from pydbsrt.tools.imghash import gen_signed_int64_hash

# https://pypi.org/project/waiting/

pytest_plugins = ["docker_compose"]


@pytest.mark.asyncio
async def test_import_binary_img_hash_to_db_async(conn, resource_phash_path):
    binary_img_hash_file = resource_phash_path("big_buck_bunny_trailer_480p.phash")
    media_id, nb_frames_inserted = await import_binary_img_hash_to_db_async(binary_img_hash_file)
    # nb binary hashes in input file
    expected_frames_inserted = len(list(gen_signed_int64_hash(binary_img_hash_file.open("rb"))))
    assert media_id == 1
    assert nb_frames_inserted == expected_frames_inserted

    media_id, nb_frames_inserted = await import_binary_img_hash_to_db_async(binary_img_hash_file)
    assert media_id == 1
    assert nb_frames_inserted == 0


@pytest.mark.asyncio
async def test_agen_p_hash_from_media_in_db(conn, resource_phash_path):
    binary_img_hash_file = resource_phash_path("big_buck_bunny_trailer_480p.phash")
    media_id, nb_frames_inserted = await import_binary_img_hash_to_db_async(binary_img_hash_file)

    media_hash = hashfile(binary_img_hash_file, hexdigest=True)

    media_phashes_in_db_with_limit = [p_hash async for p_hash in agen_p_hash_from_media_in_db(media_hash, limit=32)]
    assert len(media_phashes_in_db_with_limit) == 32
    assert media_phashes_in_db_with_limit == sorted(
        media_phashes_in_db_with_limit, key=lambda phash_media: phash_media.frame_offset
    )

    media_all_phashes_in_db = [p_hash async for p_hash in agen_p_hash_from_media_in_db(media_hash)]
    assert len(media_all_phashes_in_db) == nb_frames_inserted

    no_media_in_db = [p_hash async for p_hash in agen_p_hash_from_media_in_db("")]
    assert len(no_media_in_db) == 0
