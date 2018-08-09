"""

"""
import bitstring
from collections import defaultdict
from hashlib import md5
import imageio
from itertools import islice
from math import log, e
import numpy as np
from pathlib import Path
from tqdm import tqdm
#
from pydbsrt.tools.videoreader import VideoReader
from pydbsrt.tools.videofingerprint import VideoFingerprint
from pydbsrt.tools.subreader import SubReader
from pydbsrt.tools.subfingerprint import SubFingerprints
from pydbsrt.tools.importantframefingerprint import ImportantFrameFingerprints
from pydbsrt.tools.ffmpeg_tools.ffmeg_extract_frame import ffmpeg_imghash_generator
from pydbsrt.tools.imghash import imghash_to_bitarray


def show_fingerprints(vreader):
    #
    gen_fingerprint = VideoFingerprint(vreader)
    try:
        for i, fp in enumerate(islice(gen_fingerprint, 10)):
            print(f"id frame: {i} - fingerprint: {fp}")
    except:
        pass


def show_subtitles_fingerprints(vreader, srt_path):
    # map to group fingerprints by index subtitles (for convenience)
    map_index_subtitles = defaultdict(int)
    for index_subtitle, id_frame, fp in SubFingerprints(
            subreader=SubReader(srt_path),
            vfp=VideoFingerprint(vreader),
    ):
        if map_index_subtitles[index_subtitle] == 0:
            print(f"\nindex subtitle: {index_subtitle} - first frame: {id_frame}", end='')
        if map_index_subtitles[index_subtitle] % 8 == 0:
            print("")
        # https://stackoverflow.com/questions/493386/how-to-print-without-newline-or-space
        print(f"{fp} ", end='')
        map_index_subtitles[index_subtitle] += 1


def entropy2(labels, base=None):
    """ Computes entropy of label distribution. """

    n_labels = len(labels)

    if n_labels <= 1:
        return 0

    value, counts = np.unique(labels, return_counts=True)
    probs = counts / n_labels
    n_classes = np.count_nonzero(probs)

    if n_classes <= 1:
        return 0

    ent = 0.

    # Compute entropy
    base = e if base is None else base
    for i in probs:
        ent -= i * log(i, base)

    return ent


def show_important_frames_fingerprints(vreader):
    """
    Compute Important Frames (from fingerprint analyze) and extract frames images from them.

    :param vreader:
    :return:
    """
    gen_if_fingerprint = ImportantFrameFingerprints(
        VideoFingerprint(vreader),
        threshold_distance=32,
        threshold_nonzero=16,   # for removing blank (black) frames
    )

    # try:
    #     for i, (fp, id_frame) in enumerate(islice(gen_if_fingerprint, 16)):
    #         print(
    #             f"import frame #{i}"
    #             f" - id_frame: {id_frame}"
    #             f" - fingerprint: {fp}"
    #             f" - Shannon's entropy: {entropy2(list(str(fp)))}"
    #         )
    # except:
    #     pass
    export_path = Path('/tmp/important_frames_fingerprints')
    export_path.mkdir(exist_ok=True)
    for fp, id_frame in gen_if_fingerprint:
        print(
            f" - id_frame: {id_frame}"
            f" - fingerprint: {fp}"
            f" - Shannon's entropy: {entropy2(list(str(fp)))}"
        )
        frame = vreader.reader.get_data(id_frame)
        imageio.imwrite(export_path.joinpath(f'{id_frame}.jpg'), frame)


def export_fingerprints(
        input_media_path: Path
):
    """

    :param input_media_path:
    :return:

    {'plugin': 'ffmpeg', 'nframes': 189292, 'ffmpeg_version': '4.0.2-1 built with gcc 7 (Debian 7.3.0-26)',
     'fps': 23.98, 'source_size': (1920, 804), 'size': (1920, 804), 'duration': 7893.76}
    189260it [04:54, 641.88it/s]
    => ~ x25.675

    """
    export_path = Path('/tmp/imghash')
    export_path.mkdir(exist_ok=True)
    # https://stackoverflow.com/questions/38175170/python-md5-cracker-typeerror-object-supporting-the-buffer-api-required
    export_fp = export_path.joinpath(f"{md5(str(input_media_path).encode()).hexdigest()}.ba")
    with open(export_fp, 'wb') as fp:
        for imghash in tqdm(ffmpeg_imghash_generator(str(input_media_path))):
            ba_imghash = imghash_to_bitarray(imghash)
            fp.write(ba_imghash.bytes)


def import_fingerprints(
        input_fingerprints_path: Path
):
    """

    :param input_fingerprints_path:
    :return:
    """
    chunk_nb_frames = 2048  # for 8k of fingerprints
    chunk_nb_bytes_to_read = chunk_nb_frames << 3   # * 8
    # https://stackoverflow.com/questions/1035340/reading-binary-file-and-looping-over-each-byte
    print(f"Reading imghash file: {input_fingerprints_path} ...")
    with open(input_fingerprints_path, "rb") as f:
        while True:
            chunk = f.read(chunk_nb_bytes_to_read)
            chunk_nb_imghash = len(chunk) >> 3  # // 8
            if chunk_nb_imghash:
                # https://pythonhosted.org/bitstring/packing.html#compact-format
                for int64_imghash in bitstring.BitArray(chunk).unpack(fmt=f'>{chunk_nb_imghash}Q'):
                    yield hex(int64_imghash)
            else:
                break


def main():
    # root_path = Path('data/')
    # media_path = root_path.joinpath('big_buck_bunny_trailer_480p.webm')
    # root_path = Path("/opt/screenpulse_backup/data/backup/CHANNEL_2038")
    # media_path = root_path.joinpath('extract_2038_20170802_231535.mp4')
    root_path = Path("/home/latty/Vidéos/Mission Impossible Rogue Nation (2015) [1080p]/")
    media_path = root_path.joinpath("Mission.Impossible.Rogue.Nation.2015.1080p.BluRay.x264.YIFY.[YTS.AG].mp4")

    vreader = VideoReader(media_path)
    print(vreader.metadatas)

    # show_fingerprints(vreader)
    #
    # show_subtitles_fingerprints(vreader, srt_path=root_path.joinpath('subtitles.srt'))

    # show_important_frames_fingerprints(vreader)

    # export_fingerprints(media_path)

    map_imghash_occurency = defaultdict(int)
    for i, imghash in tqdm(enumerate(import_fingerprints(Path("/tmp/imghash/2cf8d538818fef16a65925ad55d0b1bf.ba")))):
        print(f"#{i} - {imghash}")
        map_imghash_occurency[imghash] += 1
    # print(pformat(map_imghash_occurency))
    print(f"Nb distinct imghash: {len(list(map_imghash_occurency.keys()))}")


if __name__ == '__main__':
    main()
