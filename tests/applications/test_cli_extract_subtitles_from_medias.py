"""
https://docs.pytest.org/en/stable/tmpdir.html
"""
from applications import extract_subtitles_from_medias


def test_extract_subtitles_from_medias(cli_runner):
    root_dir_for_finding_medias = "./data/"
    # FIXME: need a video with encoded subtitles
    glob_for_finding_medias = "**/big_buck_bunny_*.webm"
    default_relative_path_export_for_subtitles = "SRT"
    result = cli_runner.invoke(
        extract_subtitles_from_medias,
        args=" ".join(
            (
                f"{str(root_dir_for_finding_medias)}",
                f"{str(glob_for_finding_medias)}",
                f"--default-relative-path-export-for-subtitles {default_relative_path_export_for_subtitles}",
            )
        ),
    )
    assert result.exit_code == 0
