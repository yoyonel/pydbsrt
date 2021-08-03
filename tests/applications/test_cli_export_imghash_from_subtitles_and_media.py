"""
https://docs.pytest.org/en/stable/tmpdir.html
"""
from pydbsrt.applications import export_imghash_from_subtitles_and_media


def test_export_imghash_from_subtitles_and_media(resource_video_path, cli_runner, tmpdir):
    p_video = resource_video_path("big_buck_bunny_trailer_480p.webm")
    p_srt = resource_video_path("big_buck_bunny_trailer_480p.en.srt")
    output_file_path = tmpdir.mkdir("phash") / f"{p_video.stem}.phash"
    result = cli_runner.invoke(
        export_imghash_from_subtitles_and_media,
        args=f"--media {str(p_video)} " f"--subtitles {str(p_srt)} " f"-o {output_file_path} ",
        catch_exceptions=False,
    )
    assert result.exit_code == 0
