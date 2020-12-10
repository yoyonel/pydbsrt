"""
"""
import itertools
import logging
import operator
from collections import defaultdict
from hashlib import md5
from itertools import islice
from math import e, log
from pathlib import Path
from pprint import pformat
from typing import Dict

import bitstring
import imageio
import numpy as np
from more_itertools import grouper
from tqdm import tqdm

from pydbsrt.tools.ffmpeg_tools.ffmeg_extract_frame import ffmpeg_imghash_generator
from pydbsrt.tools.filehash import md5_file
from pydbsrt.tools.imghash import imghash_to_bitarray
from pydbsrt.tools.importantframefingerprint import ImportantFrameFingerprints
from pydbsrt.tools.subfingerprint import SubFingerprints
from pydbsrt.tools.subreader import SubReader

#
from pydbsrt.tools.timer_profiling import _Timer
from pydbsrt.tools.tqdm_with_logger import TqdmLoggingHandler
from pydbsrt.tools.videofingerprint import VideoFingerprint
from pydbsrt.tools.videoreader import VideoReader

NodeHash = int
SrtUUID = str

logger = logging.getLogger(__name__)

medias_root_path = Path("data/")
medias_path = defaultdict(
    lambda: {
        "media": medias_root_path / "big_buck_bunny_trailer_480p.webm",
        "subtitles": medias_root_path / "subtitles.srt",
    }
)  # type: Dict[str, Union[Any, Path]]


def in_debug_mode():
    """
    https://stackoverflow.com/a/38637774
    """
    import sys

    gettrace = getattr(sys, "gettrace", None)
    return gettrace() if gettrace else False


def show_fingerprints(video_reader):
    #
    gen_fingerprint = VideoFingerprint(video_reader)
    try:
        for i, fp in enumerate(islice(gen_fingerprint, 10)):
            print(f"id frame: {i} - fingerprint: {fp}")
    except:
        pass


def show_subtitles_fingerprints(video_reader: VideoReader, srt_path: Path) -> None:
    nb_fingerprints_by_chunk = 5
    it_subfingerprints = SubFingerprints(
        subreader=SubReader(srt_path), vfp=VideoFingerprint(video_reader)
    )
    gb_subfingerprints = itertools.groupby(
        it_subfingerprints, key=operator.itemgetter(0)
    )
    for index_subtitle, it_indexed_subfingerprints in gb_subfingerprints:
        _, id_frame, fingerprint = next(it_indexed_subfingerprints)
        it_indexed_subfingerprints = itertools.chain(
            ((index_subtitle, id, fingerprint),), it_indexed_subfingerprints
        )
        print(f"* index subtitle: {index_subtitle} - first frame: {id_frame}")
        for chunk_fingerprints in grouper(
            (fingerprint for _, __, fingerprint in it_indexed_subfingerprints),
            nb_fingerprints_by_chunk,
        ):
            print(" ".join(map(str, filter(None, chunk_fingerprints))))


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

    ent = 0.0

    # Compute entropy
    for i in probs:
        ent -= i * log(i, base)

    return ent


def show_important_frames_fingerprints(
    video_reader, threshold_distance: int = 32, threshold_nonzero: int = 16,
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

    export_path = Path("/tmp/important_frames_fingerprints")
    export_path.mkdir(exist_ok=True)
    for fp, id_frame in gen_if_fingerprint:
        logger.info(
            f" - id_frame: {id_frame}"
            f" - fingerprint: {fp}"
            f" - Shannon's entropy: {entropy2(list(str(fp)))}"
        )
        #
        frame = video_reader.reader.get_data(id_frame)
        frame_export = export_path.joinpath(f"{id_frame}.jpg")
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
    export_path = Path("/tmp/img_hash")
    export_path.mkdir(exist_ok=True)
    # https://stackoverflow.com/questions/38175170/python-md5-cracker-typeerror-object-supporting-the-buffer-api-required
    export_fp = export_path.joinpath(
        f"{md5(str(input_media_path).encode()).hexdigest()}.ba"
    )
    with open(export_fp, "wb") as fp:
        for img_hash in tqdm(ffmpeg_imghash_generator(str(input_media_path))):
            ba_img_hash = imghash_to_bitarray(img_hash)
            fp.write(ba_img_hash.bytes)
    return export_fp


def import_fingerprints(input_fingerprints_path: Path) -> hex:
    """

    :param input_fingerprints_path:
    :return:
    """
    # https://stackoverflow.com/questions/1035340/reading-binary-file-and-looping-over-each-byte
    print(f"Reading img_hash file: {input_fingerprints_path} ...")
    # with open(input_fingerprints_path, "rb") as f:
    with input_fingerprints_path.open("rb") as f:
        chunk_nb_frames = 2048  # for 8k of fingerprints
        chunk_nb_bytes_to_read = chunk_nb_frames << 3  # * 8
        while True:
            chunk = f.read(chunk_nb_bytes_to_read)
            chunk_nb_img_hash = len(chunk) >> 3  # // 8
            if chunk_nb_img_hash:
                # https://pythonhosted.org/bitstring/packing.html#compact-format
                for int64_img_hash in bitstring.BitArray(chunk).unpack(
                    fmt=f">{chunk_nb_img_hash}Q"
                ):
                    yield hex(int64_img_hash)
            else:
                break


def main():
    media = medias_path["big_buck_bunny_trailer_480p"]
    media_path = media["media"]  # type: Path
    srt_path = media["subtitles"]  # type: Path

    logger.info(f"media path: {media_path}")
    logger.info(f"subtitles path: {srt_path}")

    video_reader = VideoReader(media_path)
    logger.info(f"Video reader meta data:\n{pformat(video_reader.metadatas)}")


def build_search_tree(
    srt_fp_path: Path, uuid_srt: int
) -> Tuple[hamDb.BkHammingTree, Dict, int]:
    """
    Build a search (bK)Tree from a single subtitles+fingerprints dump
    """
    # load fingerprints from srt coupling
    sub_fingerprint_rip_file = SubFingerPrintRipFile.open(srt_fp_path)
    gb_sub_fingerprints = itertools.groupby(
        sub_fingerprint_rip_file, key=lambda sub_fingerprint: sub_fingerprint.srt_index
    )
    # fingerprints_for_srt = set.union(*(
    #     set([sub_fingerprints.fingerprint for sub_fingerprints in it_indexed_sub_fingerprints])
    #     for index_subtitle, it_indexed_sub_fingerprints in gb_sub_fingerprints
    # ))
    fingerprints_for_srt = set()
    nb_fingerprints_in_srt = 0
    for index_subtitle, it_indexed_sub_fingerprints in gb_sub_fingerprints:
        fingerprints_for_seq_srt = [
            sub_fingerprints.fingerprint
            for sub_fingerprints in it_indexed_sub_fingerprints
        ]
        nb_fingerprints_in_srt += len(fingerprints_for_seq_srt)
        fingerprints_for_srt.update(set(fingerprints_for_seq_srt))

    set_fingerprints = [hex(fingerprint) for fingerprint in fingerprints_for_srt]
    if in_debug_mode():
        logger.info(
            "List of fingerprints for srt\n%s\nnb fingerprints: %d",
            pformat(set_fingerprints, compact=True, indent=4),
            len(set_fingerprints),
        )

    # build bkTree with this fingerprints and associated uuid srt
    tree = hamDb.BkHammingTree()
    map_node_id_to_srt_uuid = defaultdict(int)
    for node_id, fingerprint in tqdm(
        enumerate(set_fingerprints),
        unit=" fingerprints",
        desc="Build bkTree from fingerprints",
    ):
        node_hash = hamDb.explicitSignCast(int(fingerprint, 16))
        tree.unlocked_insert(node_hash, node_id)
        # associate node is to match attributes
        # here: we just map all node id to the uuid srt
        map_node_id_to_srt_uuid[node_id] = uuid_srt
    assert (
        len(
            tree.getWithinDistance(
                hamDb.explicitSignCast(int(set_fingerprints[16], 16)), search_distance=2
            )
        )
        == 1
    )
    return tree, map_node_id_to_srt_uuid, nb_fingerprints_in_srt


def perform_search(
    search_tree: hamDb.BkHammingTree,
    map_node_value: Dict,
    gen_fingerprint: VideoFingerprint,
    search_dist: int = 4,
):
    matching_results = defaultdict(list)
    for id_fingerprint, fingerprint in tqdm(
        enumerate(gen_fingerprint), unit=" fingerprints", desc="search fingerprint"
    ):
        searched_node_hash = hamDb.explicitSignCast(int(str(fingerprint), 16))
        matches_id_node = list(
            search_tree.getWithinDistance(searched_node_hash, search_dist)
        )
        for uuid_srt_found in {
            map_node_value[match_id_node] for match_id_node in matches_id_node
        }:
            matching_results[uuid_srt_found].append(id_fingerprint)
    return matching_results


@dataclass
class SearchTree:
    srt_uuid = set()
    tree: hamDb.BkHammingTree = field(init=False)
    map_node_value_to_data: Dict[
        NodeHash, Dict[SrtUUID, Set[SubFingerPrintRipItem]]
    ] = field(init=False)

    def __post_init__(self):
        self.tree = hamDb.BkHammingTree()
        self.map_node_value_to_data = defaultdict(lambda: defaultdict(set))

    def update(self, sub_fingerprint_rip_file_path: Path, srt_uuid: str) -> int:
        """
        Build a search (bK)Tree from a single subtitles+fingerprints dump
        """
        # load fingerprints from SRT/Fingerprints coupling
        with _Timer("load fingerprints from SRT/Fingerprints coupling"):
            sub_fingerprint_rip_file = SubFingerPrintRipFile.open(
                sub_fingerprint_rip_file_path
            )
            gb_sub_fingerprints: Iterator[
                Tuple[int, Iterator[SubFingerPrintRipItem]]
            ] = itertools.groupby(
                sub_fingerprint_rip_file,
                key=lambda sub_fingerprint: sub_fingerprint.srt_index,
            )

        with _Timer("build mapping node hash to data"):
            map_node_hash_to_data: Dict[
                NodeHash, Set[SubFingerPrintRipItem]
            ] = defaultdict(set)
            for _index_subtitle, it_sub_fingerprint_rip_item in gb_sub_fingerprints:
                for fingerprint_rip_item in it_sub_fingerprint_rip_item:
                    node_hash = hamDb.explicitSignCast(fingerprint_rip_item.fingerprint)
                    map_node_hash_to_data[node_hash].add(fingerprint_rip_item)

        # Transfer data mapping from (node_hash -> data) to (node_value -> data)
        nb_node_inserted = 0
        # save srt_uuid if new coupling fingerprints/srt added
        if bool(map_node_hash_to_data):
            self.srt_uuid.add(srt_uuid)
        # update new mapping fingerprint (hash) to data
        # add (eventually) new node_hash/fingerprint in search tree
        with _Timer("update new mapping fingerprint (hash) to data"):
            while map_node_hash_to_data:
                # we consume the map
                node_hash, list_fingerprint_rip_item = map_node_hash_to_data.popitem()
                node_value = node_hash
                # Is it a new node value/hash ?
                # if yes => new fingerprint store in search tree
                # if no  => update current value attach to this fingerprint
                if not self.map_node_value_to_data[node_value]:
                    self.tree.unlocked_insert(node_hash, node_value)
                    nb_node_inserted += 1
                self.map_node_value_to_data[node_value][srt_uuid].update(
                    list_fingerprint_rip_item
                )

        if in_debug_mode():
            # each subtitle+fingerprint has 1 unique result search mapping in the tree
            assert all(
                len(
                    self.tree.getWithinDistance(
                        hamDb.explicitSignCast(sub_fingerprint_rip_item.fingerprint), 0,
                    )
                )
                == 1
                for sub_fingerprint_rip_item in sub_fingerprint_rip_file
            )

            # for each result search for subtitle+fingerprint, the index frame of request subtitle+fingerprint is in the
            # result search list (store (indirectly ~ by mapping) in the search tree)
            for sub_fingerprint_rip_item in sub_fingerprint_rip_file:
                node_value = next(
                    iter(
                        self.tree.getWithinDistance(
                            hamDb.explicitSignCast(
                                sub_fingerprint_rip_item.fingerprint
                            ),
                            0,
                        )
                    )
                )
                for (
                    response_uuid_srt,
                    list_fingerprint_rip_item,
                ) in self.map_node_value_to_data[node_value].items():
                    assert response_uuid_srt in self.srt_uuid
                    # assert sub_fingerprint_rip_item.frame_index in [
                    #     fingerprint_rip_item.frame_index
                    #     for fingerprint_rip_item in list_fingerprint_rip_item
                    # ]

        return nb_node_inserted


@dataclass(frozen=True)
class Media:
    media_path: Path
    srt_path: Path


map_srt_uuid_to_media: Dict[SrtUUID, Media] = dict()


def update_search_tree(
    search_tree: SearchTree,
    media_path: Path,
    srt_path: Path,
    export_root_dir: Path = Path("/tmp/"),
    force_export: bool = False,
):
    logger.info(f"media path: {media_path}")
    logger.info(f"subtitles path: {srt_path}")

    srt_md5_hexdigest = md5_file(srt_path)
    export_path = (export_root_dir / srt_path.stem).with_suffix(
        ".{}.srt_fingerprint".format(srt_md5_hexdigest)
    )

    if force_export or not export_path.exists():
        it_imghash = ffmpeg_imghash_generator(str(media_path))
        with _Timer("export subtitles join with fingerprints"):
            export_subtitles_fingerprints(
                it_imghash, subtitles_path=srt_path, export_path=export_path
            )

    with _Timer("update search tree"):
        nb_nodes_inserted = search_tree.update(export_path, srt_md5_hexdigest)
        map_srt_uuid_to_media[srt_md5_hexdigest] = Media(media_path, srt_path)
        logger.info("nb_nodes_inserted: %d", nb_nodes_inserted)
        logger.info(
            "nb subtitles (file) inserted in search tree: %d", len(search_tree.srt_uuid)
        )


def find_video_sequence_in_tree(
    search_tree: SearchTree,
    it_imghash: Iterator[imagehash.ImageHash],
    search_dist: int = 2,
):
    """
    https://trac.ffmpeg.org/wiki/Seeking
    https://www.bogotobogo.com/FFMpeg/ffmpeg_thumbnails_select_scene_iframe.php
    https://docs.python.org/2/library/collections.html#counter-objects
    """
    matching_results = defaultdict(lambda: defaultdict(set))

    # test search tree is not empty
    try:
        search_tree.tree.getWithinDistance(hamDb.explicitSignCast(0), 0)
    except ValueError:
        return

    with _Timer("Fingerprinting request media"):
        it_id_imghash_imghash = list(
            tqdm(
                enumerate(it_imghash),
                unit=" image hash",
                desc="build img hash from request media",
            )
        )

    with _Timer("Processing search (in tree)"):
        for id_imghash, imghash in it_id_imghash_imghash:
            searched_node_hash = hamDb.explicitSignCast(int(str(imghash), 16))
            for node_hash in search_tree.tree.getWithinDistance(
                searched_node_hash, search_dist
            ):
                for (
                    srt_uuid,
                    sub_fingerprint_rip_items,
                ) in search_tree.map_node_value_to_data[node_hash].items():
                    gb_sub_fingerprint_rip_item = itertools.groupby(
                        sub_fingerprint_rip_items,
                        key=lambda sub_fingerprint_rip_item: sub_fingerprint_rip_item.srt_index,
                    )
                    for (
                        srt_index,
                        it_sub_fingerprint_rip_items,
                    ) in gb_sub_fingerprint_rip_item:
                        matching_results[srt_uuid][srt_index].update(
                            [
                                sub_fingerprint_rip_item.frame_index
                                for sub_fingerprint_rip_item in it_sub_fingerprint_rip_items
                            ]
                        )
    return matching_results


def main():
    search_tree = SearchTree()

    root_media_path = Path("/home/latty/NAS/tvshow/Silicon Valley/S04/")
    logger.debug("root_media_path: %s", root_media_path)

    export_root_dir = Path.home() / "tmp/"
    assert os.access(str(export_root_dir), os.W_OK)
    logger.debug("export_root_dir: %s", export_root_dir)

    with _Timer("[global] build search tree"):
        for media_path in islice(root_media_path.glob("*.mkv"), 10):
            try:
                srt_path = list(root_media_path.glob(media_path.stem + ".en.srt"))[0]
            except IndexError:
                continue
            update_search_tree(
                search_tree, media_path, srt_path, export_root_dir=export_root_dir
            )

    media_path = export_root_dir / "cut.mkv"
    logger.info("media_path: %s", media_path)
    it_imghash = ffmpeg_imghash_generator(str(media_path))
    matching_results = find_video_sequence_in_tree(
        search_tree, it_imghash, search_dist=4
    )
    results = {
        uuid_srt: {
            "nb_srt_matched": len(srt_matched.keys()),
            "total_fp_matched": len(
                list(itertools.chain.from_iterable(srt_matched.values()))
            ),
            "media": map_srt_uuid_to_media[uuid_srt],
        }
        for uuid_srt, srt_matched in matching_results.items()
    }
    logger.info("Matching results:\n%s", pformat(results))

    # video_reader = VideoReader(media_path)
    # logger.info(f"Video reader meta data:\n{pformat(video_reader.metadata)}")

    # frame_reader = ffmpeg_frame_generator(str(media_path))
    # frames = list(frame_reader)
    # show_fingerprints(video_reader)
    #
    show_subtitles_fingerprints(video_reader, srt_path=srt_path)

    # show_important_frames_fingerprints(video_reader, threshold_distance=4)

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
    logger.setLevel(logging.DEBUG)
    tqdm_logging_handler = TqdmLoggingHandler()
    formatter = logging.Formatter(
        "%(asctime)s - %(name)s - %(levelname)s - %(message)s"
    )
    tqdm_logging_handler.setFormatter(formatter)
    logger.addHandler(tqdm_logging_handler)


if __name__ == "__main__":
    init_logger()
    main()
