"""
https://docs.pytest.org/en/stable/tmpdir.html
"""
import math
from io import FileIO
from pathlib import Path
from typing import Iterator
import pytest

from pydbsrt.applications.export_imghash_from_subtitles_and_media import (
    export_imghash_from_subtitles_and_media,
)
from pydbsrt.tools.imghash import bytes_to_signed_int64, signed_int64_to_str_binary
from services.reader_frames import build_reader_frames
from tools.ffmpeg_tools.ffmeg_extract_frame import rawframe_to_imghash
from tools.subfingerprint import subriptime_to_frame, SubFingerprints
from tools.subreader import SubReader


@pytest.fixture()
def resource_video_path():
    """"""

    def _resource_video_path(video_filename: str):
        p_rvp = Path(f"data/{video_filename}")
        assert p_rvp.exists()
        return p_rvp

    return _resource_video_path


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

    # count the number of binary imghashes export to the (binary file) output
    with output_file_path.open("rb") as fo:
        str_binary_hashes = list(
            map(signed_int64_to_str_binary, gen_signed_int64_hash(fo))
        )
    nb_binary_hashes = len(str_binary_hashes)

    # build readers (subtitles and (raw) frames)
    sub_reader = SubReader(p_srt)
    # TODO: construct a FrameReader class (iterator) utility (instead of functional approach)
    frame_reader, _meta = build_reader_frames(p_video)

    # compare nb output imghashes against the number of frames captured by subtitles
    # in this case, the reference source are subtitles join to (video) frames
    #
    # build object joining subtitles and images hashes (fingerprints)
    it_sub_fingerprints = SubFingerprints(
        sub_reader=sub_reader, imghash_reader=map(rawframe_to_imghash, frame_reader)
    )
    nb_fingerprints = len(list(it_sub_fingerprints))
    #
    assert nb_binary_hashes == nb_fingerprints

    # compare nb output imghashes against the number of frames captured by subtitles
    # in this case, the reference source is only subtitles (not the video)
    # iter on subtitles generator
    sub_reader = SubReader(p_srt)
    it_srt_timecodes = ((subtitle.start, subtitle.end) for subtitle in sub_reader)
    # Union des intervalles (définis par les timecodes des sous-titres) de frames
    # https://docs.python.org/2/library/stdtypes.html#frozenset.union
    imghash_from_srt = set().union(
        *(
            range(
                subriptime_to_frame(tc_start, cast_to_int=math.floor),
                subriptime_to_frame(tc_end, cast_to_int=math.ceil) + 1,
            )
            for tc_start, tc_end in it_srt_timecodes
        )
    )
    assert nb_binary_hashes == len(imghash_from_srt)
