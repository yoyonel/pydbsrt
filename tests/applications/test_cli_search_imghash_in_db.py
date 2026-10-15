from typing import Final

from click.testing import CliRunner

from pydbsrt.applications.search_imghash_in_db import search_imghash_in_db


def test_search_imghash_in_db(phash_from_media, aio_insert_phash_into_db, resource_video_path):
    distance: Final = 0
    media_id, nb_frames_inserted = aio_insert_phash_into_db
    runner = CliRunner()
    result = runner.invoke(
        search_imghash_in_db,
        args=" ".join((f"--phash_file {str(phash_from_media)}", f"--distance {distance}")),
        catch_exceptions=False,
    )
    assert result.exit_code == 0
    assert result.stdout.count("ResultSearchRecord") == nb_frames_inserted
    assert result.stdout.count(f"media_id={media_id}") >= nb_frames_inserted
