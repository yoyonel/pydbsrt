import tempfile
from pathlib import Path

from pydbsrt.applications import retarget_subtitles


def test_retarget_subtitles(big_buck_bunny_trailer, big_buck_bunny_trailer_srt, test_data_dir, cli_runner, tmpdir):
    p_ref_media = big_buck_bunny_trailer
    p_ref_srt = big_buck_bunny_trailer_srt
    p_target_media = big_buck_bunny_trailer

    result = cli_runner.invoke(
        retarget_subtitles,
        args=f"--ref_subtitles {p_ref_srt} "
        f"--ref_media {p_ref_media} "
        f"--target_media {p_target_media} "
        f"--phash_dir {test_data_dir}",
        catch_exceptions=False,
    )
    assert result.exit_code == 0

    output_retarget_subtitles = (Path(tempfile.gettempdir()) / p_target_media.stem).with_suffix(".srt")
    assert output_retarget_subtitles.exists()
