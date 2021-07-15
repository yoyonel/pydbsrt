import math

from services.extended_subtitles import export_extended_subtitles
from tools.imghash import signed_int64_to_str_binary
from tools.subfingerprint import subriptime_to_frame
from tools.subreader import SubReader

from tests.applications.test_cli_export_imghash_from_subtitles_and_media import (
    gen_signed_int64_hash,
)


def test_export_extended_subtitles(resource_video_path, tmpdir):
    p_video = resource_video_path("big_buck_bunny_trailer_480p.webm")
    p_srt = resource_video_path("big_buck_bunny_trailer_480p.en.srt")
    output_file_path = tmpdir.mkdir("phash") / f"{p_video.stem}.phash"

    assert not output_file_path.exists()
    output_file_exported = export_extended_subtitles(p_srt, p_video, output_file_path)
    assert output_file_exported.exists()

    # count the number of binary imghashes export to the (binary file) output
    with output_file_path.open("rb") as fo:
        str_binary_hashes = list(
            map(signed_int64_to_str_binary, gen_signed_int64_hash(fo))
        )
    nb_binary_hashes = len(str_binary_hashes)

    # compare nb output imghashes against the number of frames captured by subtitles
    # in this case, the reference source is only subtitles (not the video)
    # iter on subtitles generator
    sub_reader = SubReader(p_srt)
    it_sub_timecodes = ((subtitle.start, subtitle.end) for subtitle in sub_reader)
    # Union des intervalles (définis par les timecodes des sous-titres) de frames
    # https://docs.python.org/2/library/stdtypes.html#frozenset.union
    imghash_from_srt = set().union(
        *(
            range(
                subriptime_to_frame(tc_start, cast_to_int=math.floor),
                subriptime_to_frame(tc_end, cast_to_int=math.ceil) + 1,
            )
            for tc_start, tc_end in it_sub_timecodes
        )
    )
    assert nb_binary_hashes == len(imghash_from_srt)
