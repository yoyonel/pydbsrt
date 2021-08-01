import pytest
from click.testing import CliRunner

from pydbsrt.applications import recognize_media
from pydbsrt.applications.recognize_media import console
from pydbsrt.services.db_frames import import_binary_img_hash_to_db_async


@pytest.fixture
@pytest.mark.asyncio
async def aio_insert_phash_into_db(conn, resource_phash_path):
    resource_name = "big_buck_bunny_trailer_480p.phash"
    binary_img_hash_file = resource_phash_path(resource_name)
    media_id, nb_frames_inserted = await import_binary_img_hash_to_db_async(binary_img_hash_file)
    assert media_id == 1
    assert nb_frames_inserted


def test_cli_recognize_media(aio_insert_phash_into_db, resource_video_path):
    resource_video_name = "big_buck_bunny_trailer_480p"
    p_video = resource_video_path(f"{resource_video_name}.webm")
    runner = CliRunner()
    # https://rich.readthedocs.io/en/stable/reference/console.html#rich.console.Console.capture
    with console.capture() as capture:
        result = runner.invoke(
            recognize_media,
            args=" ".join((f"--media {str(p_video)}", "--output_format CSV")),
        )
    assert result.exit_code == 0, f"{str(result.exception)=}{result.exc_info=}"
    console_output = capture.get()
    assert "True," in console_output
    assert resource_video_name in console_output
