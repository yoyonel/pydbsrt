import pytest
from applications import recognize_media
from services.db_frames import import_binary_img_hash_to_db_async


@pytest.mark.skip(
    "FIXME: need to wait for database initialization done (asynchronously) before launching click command"
)
@pytest.mark.asyncio
async def test_cli_recognize_media(
    conn, event_loop, cli_runner, resource_video_path, resource_phash_path
):
    binary_img_hash_file = resource_phash_path("big_buck_bunny_trailer_480p.phash")
    media_id, nb_frames_inserted = await import_binary_img_hash_to_db_async(
        binary_img_hash_file
    )
    # FIXME: need to wait for database initialization done (asynchronously) before launching click command

    p_video = resource_video_path("big_buck_bunny_trailer_480p.webm")
    result = cli_runner.invoke(
        recognize_media,
        args=" ".join((f"--media {str(p_video)}",)),
    )
    assert result.exit_code == 0
