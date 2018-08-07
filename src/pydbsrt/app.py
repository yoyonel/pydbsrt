"""

"""
from collections import defaultdict
from pathlib import Path
import imageio
from itertools import islice
from math import log, e
import numpy as np
#
from pydbsrt.tools.videoreader import VideoReader
from pydbsrt.tools.videofingerprint import VideoFingerprint
from pydbsrt.tools.subreader import SubReader
from pydbsrt.tools.subfingerprint import SubFingerprints
from pydbsrt.tools.importantframefingerprint import ImportantFrameFingerprints


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
    Path('/tmp/important_frames_fingerprints').mkdir(exist_ok=True)
    for fp, id_frame in gen_if_fingerprint:
        print(
            f" - id_frame: {id_frame}"
            f" - fingerprint: {fp}"
            f" - Shannon's entropy: {entropy2(list(str(fp)))}"
        )
        frame = vreader.reader.get_data(id_frame)
        imageio.imwrite(f'/tmp/important_frames_fingerprints/{id_frame}.jpg', frame)


def main():
    root_path = Path('data/')
    vreader = VideoReader(media_path=root_path.joinpath('big_buck_bunny_trailer_480p.webm'))
    # root_path = Path("/opt/screenpulse_backup/data/backup/CHANNEL_2038")
    # vreader = VideoReader(media_path=root_path.joinpath('extract_2038_20170802_231535.mp4'))

    print(vreader.metadatas)

    # show_fingerprints(vreader)
    #
    show_subtitles_fingerprints(vreader, srt_path=root_path.joinpath('subtitles.srt'))

    # show_important_frames_fingerprints(vreader)


if __name__ == '__main__':
    main()
