"""
https://docs.pytest.org/en/stable/tmpdir.html
"""
from io import FileIO
from typing import Iterator

from pydbsrt.applications.export_imghash_from_subtitles_and_media import (
    export_imghash_from_subtitles_and_media,
)
from pydbsrt.tools.imghash import bytes_to_signed_int64


def gen_signed_int64_hash(fo: FileIO) -> Iterator[int]:
    ba_img_hex = fo.read(8)
    offset_frame = 0
    while ba_img_hex:
        yield bytes_to_signed_int64(ba_img_hex)
        ba_img_hex = fo.read(8)
        offset_frame += 1


def test_export_imghash_from_subtitles_and_media(
    resource_video_path, cli_runner, tmpdir
):
    p_video = resource_video_path("big_buck_bunny_trailer_480p.webm")
    p_srt = resource_video_path("big_buck_bunny_trailer_480p.en.srt")
    output_file_path = tmpdir.mkdir("phash") / f"{p_video.stem}.phash"
    result = cli_runner.invoke(
        export_imghash_from_subtitles_and_media,
        args=f"--media {str(p_video)} "
        f"--subtitles {str(p_srt)} "
        f"-o {output_file_path} ",
    )
    assert result.exit_code == 0
