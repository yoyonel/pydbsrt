"""
https://docs.pytest.org/en/stable/tmpdir.html
"""
from pydbsrt.applications.import_imghash_into_db import console, import_images_hashes_into_db
from pydbsrt.tools.imghash import gen_signed_int64_hash


def test_cli_import_imghash_into_db(conn, resource_phash_path, cli_runner):
    binary_img_hash_file = resource_phash_path("big_buck_bunny_trailer_480p.phash")
    with console.capture() as capture:
        result = cli_runner.invoke(import_images_hashes_into_db, args=f"-r {str(binary_img_hash_file)}")
    assert result.exit_code == 0
    console_output = capture.get()
    expected_frames_inserted = len(list(gen_signed_int64_hash(binary_img_hash_file.open("rb"))))
    assert f"count(frames where frames.media_id = 1)={expected_frames_inserted}" in console_output
