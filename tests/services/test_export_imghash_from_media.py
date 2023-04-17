from pydbsrt.services.imghash import export_imghash_from_media
from pydbsrt.services.reader_frames import build_reader_frames
from pydbsrt.tools.imghash import gen_signed_int64_hash, signed_int64_to_str_binary


def test_export_imghash_from_media(big_buck_bunny_trailer, tmpdir):
    p_video = big_buck_bunny_trailer
    output_file_path = tmpdir.mkdir("phash") / f"{p_video.stem}.phash"

    assert not output_file_path.exists()
    output_file_exported = export_imghash_from_media(p_video, output_file_path)
    assert output_file_exported.exists()

    # count the number of binary imghashes export to the (binary file) output
    with output_file_path.open("rb") as fo:
        str_binary_hashes = list(map(signed_int64_to_str_binary, gen_signed_int64_hash(fo)))
    nb_binary_hashes = len(str_binary_hashes)

    reader, _meta = build_reader_frames(p_video, seek_to_middle=True)
    expected_binary_hashed_count = len(list(reader))
    assert nb_binary_hashes == expected_binary_hashed_count
