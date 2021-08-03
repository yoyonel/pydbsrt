from click.testing import CliRunner

from pydbsrt.applications import recognize_media
from pydbsrt.applications.recognize_media import console


def test_cli_recognize_media(aio_insert_phash_into_db, resource_video_path):
    resource_video_name = "big_buck_bunny_trailer_480p"
    p_video = resource_video_path(f"{resource_video_name}.webm")
    runner = CliRunner()
    # https://rich.readthedocs.io/en/stable/reference/console.html#rich.console.Console.capture
    with console.capture() as capture:
        result = runner.invoke(
            recognize_media,
            args=" ".join((f"--media {str(p_video)}", "--output_format CSV")),
            catch_exceptions=False,
        )
    assert result.exit_code == 0
    console_output = capture.get()
    assert "True," in console_output
    assert resource_video_name in console_output
