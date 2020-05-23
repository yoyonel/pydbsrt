"""
"""
import itertools
import logging
import operator
import uuid
from collections import defaultdict
from functools import lru_cache
from hashlib import md5
from itertools import islice
from math import log, e
from pathlib import Path
from pprint import pformat
from typing import Dict, Any, Union, Iterable, Tuple

import bitstring
import cbktree as hamDb
import imageio
import numpy as np
from more_itertools import grouper
from tqdm import tqdm

from pydbsrt.tools.ffmpeg_tools.ffmeg_extract_frame import ffmpeg_imghash_generator
from pydbsrt.tools.imghash import imghash_to_bitarray
from pydbsrt.tools.importantframefingerprint import ImportantFrameFingerprints
from pydbsrt.tools.srt_fp_item import SubFingerPrintRipItem
from pydbsrt.tools.str_fp_file import SubFingerPrintRipFile
from pydbsrt.tools.subfingerprint import SubFingerprints
from pydbsrt.tools.subreader import SubReader
#
from pydbsrt.tools.tqdm_with_logger import TqdmLoggingHandler
from pydbsrt.tools.videofingerprint import VideoFingerprint
from pydbsrt.tools.videoreader import VideoReader

logger = logging.getLogger(__name__)

medias_root_path = Path('data/')
medias_path = defaultdict(
    lambda: {
        'media': medias_root_path / 'big_buck_bunny_trailer_480p.webm',
        'subtitles': medias_root_path / 'subtitles.srt',
    }
)  # type: Dict[str, Union[Any, Path]]


@lru_cache()
def in_debug_mode():
    """
    https://stackoverflow.com/a/38637774
    """
    import sys

    gettrace = getattr(sys, 'gettrace', None)
    return gettrace() if gettrace else False


def show_fingerprints(video_reader):
    #
    gen_fingerprint = VideoFingerprint(video_reader)
    try:
        for i, fp in enumerate(islice(gen_fingerprint, 10)):
            print(f"id frame: {i} - fingerprint: {fp}")
    except:
        pass


def show_subtitles_fingerprints(video_reader: VideoReader, subtitles_path: Path) -> None:
    nb_fingerprints_by_chunk = 5
    it_subfingerprints = SubFingerprints(sub_reader=SubReader(subtitles_path),
                                         video_fingerprint=VideoFingerprint(video_reader))
    gb_subfingerprints = itertools.groupby(it_subfingerprints, key=operator.itemgetter(0))
    for index_subtitle, it_indexed_subfingerprints in gb_subfingerprints:
        _, id_frame, fingerprint = next(it_indexed_subfingerprints)
        it_indexed_subfingerprints = itertools.chain(((index_subtitle, id, fingerprint),), it_indexed_subfingerprints)
        print(f"* index subtitle: {index_subtitle} - first frame: {id_frame}")
        for chunk_fingerprints in grouper((fingerprint for _, __, fingerprint in it_indexed_subfingerprints),
                                          nb_fingerprints_by_chunk):
            print(" ".join(map(str, filter(None, chunk_fingerprints))))


def export_subtitles_fingerprints(video_reader: VideoReader, subtitles_path: Path, export_path: Path) -> None:
    it_subfingerprints = SubFingerprints(sub_reader=SubReader(subtitles_path),
                                         video_fingerprint=VideoFingerprint(video_reader))
    # SubFingerPrintRipItem
    it_items = (
        SubFingerPrintRipItem(index, id_frame, fingerprint)
        for index, id_frame, fingerprint in tqdm(it_subfingerprints,
                                                 unit=" subfingerprints",
                                                 desc="join subtitle and frame fingerprint")
    )
    logger.info("Export Subtitles+Fingerprints into: %s", export_path)
    # consumed by Parent class: `UserList`
    SubFingerPrintRipFile(items=it_items, path=export_path).save()


def import_subtitles_fingerprints(import_path: Path):
    sub_fingerprint_rip_file = SubFingerPrintRipFile.open(import_path)
    for sub_fingerprint_rip in sub_fingerprint_rip_file:
        logger.info("subtitle.index: %d - frame.index: %d - frame.hash: %s",
                    sub_fingerprint_rip.srt_index,
                    sub_fingerprint_rip.frame_index, hex(sub_fingerprint_rip.fingerprint))


def entropy2(labels, base=e):
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
    for i in probs:
        ent -= i * log(i, base)

    return ent


def show_important_frames_fingerprints(
        video_reader,
        threshold_distance: int = 32,
        threshold_nonzero: int = 16,
):
    """
    Compute Important Frames (from fingerprint analyze) and extract frames images from them.

    :param video_reader:
    :param threshold_distance:
    :param threshold_nonzero:
    :return:
    """
    gen_if_fingerprint = ImportantFrameFingerprints(
        VideoFingerprint(video_reader),
        threshold_distance=threshold_distance,
        threshold_nonzero=threshold_nonzero,
        # for removing blank (black) frames
    )

    export_path = Path('/tmp/important_frames_fingerprints')
    export_path.mkdir(exist_ok=True)
    for fp, id_frame in gen_if_fingerprint:
        logger.info(
            f" - id_frame: {id_frame}"
            f" - fingerprint: {fp}"
            f" - Shannon's entropy: {entropy2(list(str(fp)))}"
        )
        #
        frame = video_reader.reader.get_data(id_frame)
        frame_export = export_path.joinpath(f'{id_frame}.jpg')
        logger.info(f"Export frame.id={id_frame} to '{frame_export}'")
        imageio.imwrite(frame_export, frame)


def export_fingerprints(input_media_path: Path) -> Path:
    """

    :param input_media_path:
    :return:

    {'plugin': 'ffmpeg', 'nframes': 189292, 'ffmpeg_version': '4.0.2-1 built with gcc 7 (Debian 7.3.0-26)',
     'fps': 23.98, 'source_size': (1920, 804), 'size': (1920, 804), 'duration': 7893.76}
    189260it [04:54, 641.88it/s]
    => ~ x25.675

    """
    export_path = Path('/tmp/img_hash')
    export_path.mkdir(exist_ok=True)
    # https://stackoverflow.com/questions/38175170/python-md5-cracker-typeerror-object-supporting-the-buffer-api-required
    export_fp = export_path.joinpath(f"{md5(str(input_media_path).encode()).hexdigest()}.ba")
    with open(export_fp, 'wb') as fp:
        nb_img_hash_written = 0
        for img_hash in tqdm(ffmpeg_imghash_generator(str(input_media_path))):
            ba_img_hash = imghash_to_bitarray(img_hash)
            fp.write(ba_img_hash.bytes)
            nb_img_hash_written += 1
        logger.info("Nb image hash written: %d", nb_img_hash_written)
    return export_fp


def import_fingerprints(input_fingerprints_path: Path) -> hex:
    """

    :param input_fingerprints_path:
    :return:
    """
    chunk_nb_frames = 2048  # for 8k of fingerprints
    chunk_nb_bytes_to_read = chunk_nb_frames << 3  # * 8
    # https://stackoverflow.com/questions/1035340/reading-binary-file-and-looping-over-each-byte
    print(f"Reading img_hash file: {input_fingerprints_path} ...")
    # with open(input_fingerprints_path, "rb") as f:
    with input_fingerprints_path.open('rb') as f:
        while True:
            chunk = f.read(chunk_nb_bytes_to_read)
            chunk_nb_img_hash = len(chunk) >> 3  # // 8
            if chunk_nb_img_hash:
                # https://pythonhosted.org/bitstring/packing.html#compact-format
                for int64_img_hash in bitstring.BitArray(chunk).unpack(fmt=f'>{chunk_nb_img_hash}Q'):
                    yield hex(int64_img_hash)
            else:
                break


def build_search_tree(srt_fp_path: Path, uuid_srt: int) -> Tuple[hamDb.BkHammingTree, Dict]:
    """
    Build a search (bK)Tree from a single subtitles+fingerprints dump
    """
    # load fingerprints from srt coupling
    sub_fingerprint_rip_file = SubFingerPrintRipFile.open(srt_fp_path)
    gb_subfingerprints = itertools.groupby(sub_fingerprint_rip_file,
                                           key=lambda sub_fingerprint: sub_fingerprint.srt_index)
    fingerprints_for_srt = set.union(*(
        set([subfingerprints.fingerprint for subfingerprints in it_indexed_subfingerprints])
        for index_subtitle, it_indexed_subfingerprints in gb_subfingerprints
    ))
    set_fingerprints = [hex(fingerprint) for fingerprint in fingerprints_for_srt]
    if in_debug_mode():
        logger.info(
            "List of fingerprints for srt\n%s\nnb fingerprints: %d",
            pformat(set_fingerprints, compact=True, indent=4),
            len(set_fingerprints),
        )

    # build bkTree with this fingerprints and associated uuid srt
    tree = hamDb.BkHammingTree()
    map_node_id_to_srt_uuid = defaultdict(set)
    for node_id, fingerprint in tqdm(enumerate(set_fingerprints), unit=" fingerprints",
                                     desc="Build bkTree from fingerprints"):
        node_hash = hamDb.explicitSignCast(int(fingerprint, 16))
        tree.unlocked_insert(node_hash, node_id)
        # associate node is to match attributes
        # here: we just map all node id to the uuid srt
        map_node_id_to_srt_uuid[node_id].add(uuid_srt)
    assert len(tree.getWithinDistance(hamDb.explicitSignCast(int(set_fingerprints[16], 16)), 2)) == 1
    return tree, map_node_id_to_srt_uuid


def perform_search(
        search_tree: hamDb.BkHammingTree,
        map_node_value: Dict,
        gen_fingerprint: VideoFingerprint,
        search_dist: int = 4
):
    matching_results = defaultdict(set)
    for id_fingerprint, fingerprint in enumerate(gen_fingerprint):
        matches_id_node, _nb_tree_nodes_touched = search_tree.getWithinDistance(fingerprint, search_dist)
        for match_id_node in matches_id_node:
            matching_results[map_node_value[match_id_node]].add(id_fingerprint)
    logger.info("matching_results: %s", matching_results)



def main():
    media = medias_path['big_buck_bunny_trailer_480p']
    media_path = media['media']  # type: Path
    srt_path = media['subtitles']  # type: Path

    logger.info(f"media path: {media_path}")
    logger.info(f"subtitles path: {srt_path}")

    video_reader = VideoReader(media_path)
    logger.info(f"Video reader meta data:\n{pformat(video_reader.metadata)}")

    # show_fingerprints(video_reader)
    #
    # show_subtitles_fingerprints(video_reader, subtitles_path=srt_path)
    export_path = (Path("/tmp/") / srt_path.stem).with_suffix(".srt_fingerprint")
    export_subtitles_fingerprints(video_reader, subtitles_path=srt_path, export_path=export_path)
    # import_subtitles_fingerprints(import_path=export_path)
    # show_important_frames_fingerprints(video_reader, threshold_distance=4)
    search_tree, map_node_value = build_search_tree(export_path, uuid_srt=uuid.uuid4().int & (1 << 64) - 1)

    it_subfingerprints = SubFingerprints(sub_reader=SubReader(srt_path),
                                         video_fingerprint=VideoFingerprint(video_reader))
    perform_search(search_tree, map_node_value, it_subfingerprints)

    fp_exported = export_fingerprints(media_path)
    logger.info(f"Fingerprints exported: {fp_exported}")

    map_img_hash_occurency = defaultdict(int)
    for img_hash in import_fingerprints(fp_exported):
        map_img_hash_occurency[img_hash] += 1
    logger.info(f"Nb distinct img_hash: {len(list(map_img_hash_occurency.keys()))}")

    # instance.opt
    nb_fingerprints = 25 * 2  # 1 minute
    with open("/tmp/img_hash/instance.opt", "w") as fp:
        img_hashs = set(islice(import_fingerprints(fp_exported), nb_fingerprints))
        print("\n".join(("2", str(len(img_hashs)), "64", "0", "1")), file=fp)
        for img_hash in img_hashs:
            # https://stackoverflow.com/questions/1425493/convert-hex-to-binary
            print(bin(int(img_hash, 16))[2:], file=fp)


def init_logger():
    logger.setLevel(logging.INFO)
    tqdm_logging_handler = TqdmLoggingHandler()
    formatter = logging.Formatter(
        '%(asctime)s - %(name)s - %(levelname)s - %(message)s')
    tqdm_logging_handler.setFormatter(formatter)
    logger.addHandler(tqdm_logging_handler)


if __name__ == '__main__':
    init_logger()
    main()
