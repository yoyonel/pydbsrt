import re

from pydbsrt.applications import show_imghash_from_subtitles_and_media
from pydbsrt.services.extended_subtitles import console


def test_show_imghash_from_subtitles_and_media(cli_runner, big_buck_bunny_trailer, big_buck_bunny_trailer_srt):
    p_video = big_buck_bunny_trailer
    p_subtitles = big_buck_bunny_trailer_srt
    with console.capture() as capture:
        result = cli_runner.invoke(
            show_imghash_from_subtitles_and_media,
            args=f"-m {str(p_video)} -s {str(p_subtitles)}",
            catch_exceptions=False,
        )
    assert result.exit_code == 0
    console_output = capture.get()

    # https://regex101.com/r/Awetfu/1
    regex = r"\* index subtitle: (?P<i_subtitle>\d*) - first frame: (?P<i_frame>\d*)"
    matches = re.finditer(regex, console_output, re.MULTILINE)
    extracted_index_subtitles_frame = [(match["i_subtitle"], match["i_frame"]) for match in matches]
    expected_index_subtitles_frame = [
        ('1', '0'),
        ('2', '150'),
        ('3', '272'),
        ('4', '409'),
        ('5', '573'),
        ('6', '667'),
        ('7', '750'),
        ('8', '778'),
    ]
    assert extracted_index_subtitles_frame == expected_index_subtitles_frame
