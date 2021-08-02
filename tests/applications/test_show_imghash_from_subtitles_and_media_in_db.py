import re

from pydbsrt.applications import show_imghash_from_subtitles_and_media_in_db
from pydbsrt.services.extended_subtitles import console


def test_cli_show_imghash_from_subtitles_and_media_in_db(
    aio_insert_phash_into_db,
    phash_from_media,
    cli_runner,
    resource_video_path,
):
    resource_video_name = "big_buck_bunny_trailer_480p"
    p_subtitles = resource_video_path(f"{resource_video_name}.en.srt")

    with console.capture() as capture:
        result = cli_runner.invoke(
            show_imghash_from_subtitles_and_media_in_db, args=f"-r {str(phash_from_media)} -s {str(p_subtitles)}"
        )
    assert result.exit_code == 0, f"{str(result.exception)=}{result.exc_info=}"
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