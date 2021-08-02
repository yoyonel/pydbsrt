"""
https://docs.pytest.org/en/stable/tmpdir.html
"""
from pathlib import Path

import distance
import imagehash
import numpy as np
import pytest
from PIL import Image

from pydbsrt.applications import export_imghash_from_media
from pydbsrt.tools.imghash import gen_signed_int64_hash, imghash_to_64bits, signed_int64_to_str_binary


@pytest.fixture()
def resource_colors_video_path():
    """
    https://stackoverflow.com/a/47704630 (How to generate a 2hour-long blank video)
    ffmpeg commands:
        - `ffmpeg -y -t 1 -f lavfi -i color=c=white:s=32x32 -c:v libx264 -tune stillimage -pix_fmt yuv420p white_frames.mp4`
        - `ffmpeg -y -t 1 -f lavfi -i color=c=black:s=32x32 -c:v libx264 -tune stillimage -pix_fmt yuv420p black_frames.mp4`
    TODO: tester d'autres encodages !
    """
    return lambda color_frame: Path(f"data/{color_frame}_frames.mp4")


def test_cli_export_imghash(resource_video_path, cli_runner, tmpdir):
    resource_video_name = "big_buck_bunny_trailer_480p"
    p_video = resource_video_path(f"{resource_video_name}.webm")
    binary_img_hash_file = tmpdir.mkdir("phash") / f"{resource_video_name}.phash"
    result = cli_runner.invoke(export_imghash_from_media, args=f"-r {str(p_video)} -o {binary_img_hash_file}")
    assert result.exit_code == 0


def test_cli_export_imghash_from_white_frames(resource_colors_video_path, cli_runner, tmpdir):
    p_video = resource_colors_video_path("white")
    output_file_path = tmpdir.mkdir("phash") / f"{p_video.stem}.phash"
    result = cli_runner.invoke(export_imghash_from_media, args=f"-r {str(p_video)} -o {output_file_path}")
    assert result.exit_code == 0

    ndarray_white_frame = np.ndarray(dtype=np.uint8, shape=(32, 32))
    ndarray_white_frame.fill(255)
    phash_frame = imagehash.phash(Image.fromarray(ndarray_white_frame))
    expected_str_bits_hash_from_frame = imghash_to_64bits(phash_frame)
    with output_file_path.open("rb") as fo:
        str_binary_hashes = map(signed_int64_to_str_binary, gen_signed_int64_hash(fo))
        diffs = [
            distance.hamming(expected_str_bits_hash_from_frame, str_binary_hash)
            for str_binary_hash in str_binary_hashes
        ]
        assert sum(diffs) == 0


@pytest.mark.xfail(raises=ValueError)
def test_cli_export_imghash_from_black_frames(resource_colors_video_path, cli_runner, tmpdir):
    """
    Test pour montrer que l'algo d'hashing (et reconnaissance) ne fonctionne pas dans certain cas de figure ("pathologique").
    Par exemple, une vidéo composée de frame purement "noire", avec la compression (même élevée) va produire quelques artefacts,
    cad les frames contiendront aléatoirement (ou presque) quelques pixels non totalement noirs.
    Ces quelques pixels non noirs vont casser l'uniformité de la frame et engendrés des "ondes" spectrale dans l'analyse (décomposition) DCT.
    Ces "ondes" impliqueront des hashes non nul, ce qui était les hashes attendus.
    """
    p_video = resource_colors_video_path("black")
    output_file_path = tmpdir.mkdir("phash") / f"{p_video.stem}.phash"
    result = cli_runner.invoke(export_imghash_from_media, args=f"-r {str(p_video)} -o {output_file_path}")
    assert result.exit_code == 0

    ndarray_black_frame = np.ndarray(dtype=np.uint8, shape=(32, 32))
    ndarray_black_frame.fill(0)
    phash_frame = imagehash.phash(Image.fromarray(ndarray_black_frame))
    expected_str_bits_hash_from_frame = imghash_to_64bits(phash_frame)
    with output_file_path.open("rb") as fo:
        str_binary_hashes = list(map(signed_int64_to_str_binary, gen_signed_int64_hash(fo)))
        diffs = [
            distance.hamming(expected_str_bits_hash_from_frame, str_binary_hash)
            for str_binary_hash in str_binary_hashes
        ]
        sum_diffs = sum(diffs)
        if sum_diffs != 0:
            # 5 bits de différences sur chaque frame (lié à un encodage spécifique)
            assert sum_diffs >= 25 * 5
            ValueError(f"{sum_diffs=} not equal to 0 !")
