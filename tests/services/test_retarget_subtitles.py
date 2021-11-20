import re
from itertools import chain
from pathlib import Path
from typing import Final, Iterator

import pytest

from pydbsrt.services import retarget_subtitles_async
from pydbsrt.services.retarget_srt import console


@pytest.mark.asyncio
async def test_retarget_subtitles(
    big_buck_bunny_trailer_srt,
    resource_phash_path,
    tmpdir,
):
    media_name = "big_buck_bunny_trailer_480p"
    ref_subtitles = big_buck_bunny_trailer_srt
    ref_phash_file = resource_phash_path(f"{media_name}.phash")
    # Generate an another (temp) phash file for target => random_phash + ref_phash
    nb_random_seconds = 5
    nb_random_phash: Final[int] = 24 * nb_random_seconds
    it_target_img_hash: Iterator[bytes] = chain(
        (open("/dev/urandom", "rb").read(8) for _ in range(nb_random_phash)), ref_phash_file.open("rb")
    )
    target_phash_file = Path(tmpdir / f"target_{ref_phash_file.stem}.phash")
    with target_phash_file.open("ab") as fo:
        fo.write(b"".join(it_target_img_hash))

    with console.capture() as capture:
        output_retarget_subtitles = await retarget_subtitles_async(
            ref_subtitles,
            ref_phash_file,
            target_phash_file,
        )
        assert output_retarget_subtitles.exists()
    console_output = capture.get()
    regex = r"Shift subtitles seconds = (?P<nb_frames>\d*) frames ~ (?P<nb_seconds>\d*\.*\d+) seconds"
    matches = list(re.finditer(regex, console_output, re.MULTILINE))
    assert len(matches) == 1
    assert matches[0]["nb_frames"] == str(nb_random_phash)
    assert matches[0]["nb_seconds"] == f"{float(nb_random_seconds):.4f}"
