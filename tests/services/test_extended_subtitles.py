import math
from functools import lru_cache
from typing import Iterable, Iterator, Tuple

from pysrt import SubRipTime

from pydbsrt.services.extended_subtitles import export_extended_subtitles, read_extended_subtitles
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


def test_export_extended_subtitles(resource_video_path, tmpdir):
    p_video = resource_video_path("big_buck_bunny_trailer_480p.webm")
    p_srt = resource_video_path("big_buck_bunny_trailer_480p.en.srt")
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


def test_read_extended_subtitles(resource_video_path, tmpdir):
    p_video = resource_video_path("big_buck_bunny_trailer_480p.webm")
    p_srt = resource_video_path("big_buck_bunny_trailer_480p.en.srt")
    output_file_path = tmpdir.mkdir("phash") / f"{p_video.stem}.phash"

    output_file_exported = export_extended_subtitles(p_srt, p_video, output_file_path)

    signed_int64_img_hashes = list(read_extended_subtitles(output_file_exported))

    # compare ...
    assert len(signed_int64_img_hashes) == _compute_nb_frames_from_srt(p_srt)


# def test_show_subtitles_fingerprints():
#     # TODO: write utest ^^
#     ...
