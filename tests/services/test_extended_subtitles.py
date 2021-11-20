import math
import re
from functools import lru_cache
from typing import Iterable, Iterator, Tuple

import pytest
from pysrt import SubRipTime

from pydbsrt.services.db_frames import agen_p_hash_from_media_in_db
from pydbsrt.services.extended_subtitles import (
    async_show_subtitles_fingerprints,
    console,
    export_extended_subtitles,
    read_extended_subtitles,
)
from pydbsrt.tools.aio_filehash import aio_hashfile
from pydbsrt.tools.imghash import gen_signed_int64_hash, signed_int64_to_str_binary
from pydbsrt.tools.subfingerprint import subriptime_to_frame
from pydbsrt.tools.subreader import SubReader


@lru_cache
def _compute_nb_frames_from_srt(p_srt) -> int:
    sub_reader = SubReader(p_srt)
    it_sub_timecodes: Iterator[Tuple[SubRipTime, SubRipTime]] = (
        (subtitle.start, subtitle.end) for subtitle in sub_reader
    )
    # Union des intervalles (définis par les timecodes des sous-titres) de frames
    # https://docs.python.org/2/library/stdtypes.html#frozenset.union
    it_range: Iterable = (
        range(
            subriptime_to_frame(tc_start, cast_to_int=math.floor),
            subriptime_to_frame(tc_end, cast_to_int=math.ceil) + 1,
        )
        for tc_start, tc_end in it_sub_timecodes
    )
    return len(set().union(*it_range))


def test_export_extended_subtitles(big_buck_bunny_trailer, big_buck_bunny_trailer_srt, tmpdir):
    p_video = big_buck_bunny_trailer
    p_srt = big_buck_bunny_trailer_srt
    output_file_path = tmpdir.mkdir("phash") / f"{p_video.stem}.phash"

    assert not output_file_path.exists()
    output_file_exported = export_extended_subtitles(p_srt, p_video, output_file_path)
    assert output_file_exported.exists()

    # count the number of binary imghashes export to the (binary file) output
    with output_file_path.open("rb") as fo:
        str_binary_hashes = list(map(signed_int64_to_str_binary, gen_signed_int64_hash(fo)))
    nb_binary_hashes = len(str_binary_hashes)

    # compare nb output imghashes against the number of frames captured by subtitles
    # in this case, the reference source is only subtitles (not the video)
    # iter on subtitles generator
    assert nb_binary_hashes == _compute_nb_frames_from_srt(p_srt)


def test_read_extended_subtitles(big_buck_bunny_trailer, big_buck_bunny_trailer_srt, tmpdir):
    p_video = big_buck_bunny_trailer
    p_srt = big_buck_bunny_trailer_srt
    output_file_path = tmpdir.mkdir("phash") / f"{p_video.stem}.phash"

    output_file_exported = export_extended_subtitles(p_srt, p_video, output_file_path)

    signed_int64_img_hashes = list(read_extended_subtitles(output_file_exported))

    # compare ...
    assert len(signed_int64_img_hashes) == _compute_nb_frames_from_srt(p_srt)


# def test_show_subtitles_fingerprints():
#     # TODO: write utest ^^
#     ...


@pytest.mark.asyncio
async def test_async_show_subtitles_fingerprints(
    aio_insert_phash_into_db,
    phash_from_media,
    resource_video_path,
):
    resource_video_name = "big_buck_bunny_trailer_480p"
    p_subtitles = resource_video_path(f"{resource_video_name}.en.srt")
    binary_img_hash_file = phash_from_media

    media_hash = await aio_hashfile(binary_img_hash_file, hexdigest=True)
    agen_phash_media = agen_p_hash_from_media_in_db(media_hash)
    with console.capture() as capture:
        await async_show_subtitles_fingerprints(p_subtitles, agen_phash_media)

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
