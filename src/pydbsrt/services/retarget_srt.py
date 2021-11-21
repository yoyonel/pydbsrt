"""
"""
import itertools
import re
from collections import Counter
from dataclasses import dataclass
from itertools import groupby
from operator import itemgetter
from pathlib import Path
from statistics import StatisticsError, mode
from tempfile import gettempdir
from typing import Final, Iterator, List, Tuple

import asyncpg
import distance
import pybktree
import pysrt
from rich.console import Console

from pydbsrt.services.database import psqlDbIpAddr, psqlDbName, psqlUserName, psqlUserPass
from pydbsrt.services.matching import ResultSearch, search_phash_stream_in_db
from pydbsrt.services.reader_frames import gen_read_binary_img_hash_file
from pydbsrt.tools.aio_filehash import aio_hashfile
from pydbsrt.tools.imghash import signed_int64_to_str_binary
from pydbsrt.tools.pairwise import pairwise
from pydbsrt.tools.search_tree import Item, MatchedItem, SearchTree, item_distance
from pydbsrt.tools.subfingerprint import SubFingerprint, SubFingerprints
from pydbsrt.tools.subreader import SubReader
from pydbsrt.tools.timer_profiling import _Timer

console = Console()

# https://regex101.com/r/F4To6t/1
framerate_r = re.compile(r"/*\.(?P<framerate>\d*\.\d*)fps*\.phash$")
DEFAULT_FRAMERATE: Final[float] = 24.00
DEFAULT_THRESHOLD_DISTANCE_FOR_CUT: Final[int] = 16


@dataclass
class SearchTreeFromCuts(SearchTree):
    """ """

    threshold_distance_for_cut = DEFAULT_THRESHOLD_DISTANCE_FOR_CUT

    def __post_init__(self):
        self.tree = pybktree.BKTree(
            item_distance, gen_items_from_cuts(self.binary_img_hash_file, self.threshold_distance_for_cut)
        )


def gen_items_from_cuts(
    phash_file: Path, threshold_distance_for_cut: int = DEFAULT_THRESHOLD_DISTANCE_FOR_CUT
) -> Iterator[Item]:
    """

    :param phash_file:
    :param threshold_distance_for_cut:
    :return:
    """
    it_img_hash: Iterator[int] = (img_hash for img_hash, _, _ in gen_read_binary_img_hash_file(phash_file, 1))
    # https://docs.python.org/3.8/library/itertools.html#itertools.chain
    # a paired frames correspond to a "cut"
    # keep all paired frames with a (hamming) distance greater than a threshold (for example: 16 (different bits))
    return itertools.chain(
        *(
            (Item(s0, offset_frame), Item(s1, offset_frame + 1))
            for (offset_frame, (s0, s1)) in enumerate(pairwise(it_img_hash))
            if distance.hamming(signed_int64_to_str_binary(s0), signed_int64_to_str_binary(s1))
            > threshold_distance_for_cut
        )
    )


async def retarget_subtitles_in_db_async(
    ref_phash_file_hash: Path, ref_phash_file: Path, ref_subtitles: Path, target_phash_file_hash: Path
) -> list[tuple[int, int]]:
    """

    :param ref_phash_file_hash:
    :param ref_phash_file:
    :param ref_subtitles:
    :param target_phash_file_hash:
    :return:
    """
    async with asyncpg.create_pool(
        user=psqlUserName, password=psqlUserPass, database=psqlDbName, host=psqlDbIpAddr, command_timeout=60
    ) as pool:
        async with pool.acquire() as conn:
            found_ref_media_id = await conn.fetchval(
                """
                    SELECT
                        id
                    FROM
                        medias
                    WHERE
                        medias.media_hash = $1;
                """,
                ref_phash_file_hash,
            )
            assert found_ref_media_id
            found_target_media_id = await conn.fetchval(
                """
                    SELECT
                        id
                    FROM
                        medias
                    WHERE
                        medias.media_hash = $1;
                """,
                target_phash_file_hash,
            )
            assert found_target_media_id

            it_ref_img_hash: Iterator[int] = (
                img_hash for img_hash, _, _ in gen_read_binary_img_hash_file(ref_phash_file, found_ref_media_id)
            )

            gb_ref_sub_fingerprints: Iterator[Tuple[int, Iterator[SubFingerprint]]] = groupby(
                SubFingerprints(sub_reader=SubReader(ref_subtitles), imghash_reader=it_ref_img_hash),
                key=itemgetter("index"),
            )

            map_mcdp_diff_offsets = {}
            # TODO: add progress bar
            for index_subtitle, it_indexed_sub_fingerprints in gb_ref_sub_fingerprints:
                ref_img_hashes_sub_fp = (
                    indexed_sub_fingerprints.img_hash for indexed_sub_fingerprints in it_indexed_sub_fingerprints
                )
                # TODO: Need optimize db connection here ! Each search initialize a new db connection ! (maybe it's cached ... need to clarify !)
                results_search: ResultSearch = await search_phash_stream_in_db(
                    map(str, ref_img_hashes_sub_fp), search_distance=0
                )
                try:
                    # https://docs.python.org/3/library/statistics.html#statistics.mode
                    # "most common data point" of difference offsets
                    mcdp_diff_offsets = mode(
                        (
                            record.matches[1].frame_offset - record.matches[0].frame_offset
                            for record in results_search.records
                            if len(record.matches) == 2 and record.matches[0].media_id != record.matches[1].media_id
                        )
                    )
                    map_mcdp_diff_offsets[index_subtitle] = mcdp_diff_offsets
                except StatisticsError:
                    # No common difference offsets to compute !
                    pass
                ###############################################################################################################################################
                # [
                #     distance.hamming(*[signed_int64_to_str_binary(result_search.search_phash) for result_search in results_paired_search])
                #     for results_paired_search in zip(results_search.records[:-1], results_search.records[1:])
                # ]
                ###############################################################################################################################################
            # https://docs.python.org/3/library/collections.html#collections.Counter
            return Counter(map_mcdp_diff_offsets.values()).most_common(5)


def retarget_subtitles_with_bktree(
    ref_phash_file: Path, _ref_subtitles: Path, target_phash_file: Path
) -> list[tuple[int, int]]:
    """

    :param ref_phash_file:
    :param _ref_subtitles:
    :param target_phash_file:
    :return:
    """
    search_distance: Final[int] = 0

    with _Timer("Build BKTrees for target", func_to_log=console.print):
        target_st = SearchTreeFromCuts(target_phash_file)
        console.print(f"{target_st=}")

    with _Timer("Search differences offsets between reference and target", func_to_log=console.print):
        it_items = gen_items_from_cuts(ref_phash_file)
        # search all paired matched frames from reference and target
        it_matches: Iterator[Tuple[List[MatchedItem], List[MatchedItem]]] = filter(
            lambda matches: all(matches),
            (
                (
                    [
                        (0, ref_item),
                    ],
                    target_st.find(ref_item, search_distance),
                )
                for ref_item in it_items
            ),
        )
        # just keep the first paired frames matched (especially from target)
        it_first_matches: Iterator[Tuple[MatchedItem, MatchedItem]] = (
            (matches_in_ref[0], matches_in_target[0]) for matches_in_ref, matches_in_target in it_matches
        )
        # compute differences offsets for each paired frames from reference and target
        it_diff_offsets: Iterator[int] = (
            first_match_item_in_target.id - first_match_item_in_ref.id
            for ((_, first_match_item_in_ref), (_, first_match_item_in_target)) in it_first_matches
        )

        counter_diff_offsets = Counter(it_diff_offsets)
        nb_paired_frames_used = sum(counter_diff_offsets.values())
        nb_paired_frames_used_by_mc = counter_diff_offsets.most_common(1)[0][1]
        console.print(
            f"Nb paired frames used: {nb_paired_frames_used} frames ~ {nb_paired_frames_used / 24.00} seconds"
        )
        console.print(
            f"Percent of frames used for the most common: ~ {(nb_paired_frames_used_by_mc / nb_paired_frames_used) * 100:.2f} %"
        )
        # return 3 most common values (differences offsets) from counter
        return counter_diff_offsets.most_common(3)


async def retarget_subtitles_async(
    ref_subtitles: Path,
    ref_phash_file: Path,
    target_phash_file: Path,
) -> Path:
    ref_phash_file_hash = await aio_hashfile(ref_phash_file, hexdigest=True)
    console.print(f"{ref_phash_file_hash=}")
    target_phash_file_hash = await aio_hashfile(target_phash_file, hexdigest=True)
    console.print(f"{target_phash_file_hash=}")

    with _Timer("Find differences offsets for synchronize subtitles (reference -> target)", func_to_log=console.print):
        # [DB]
        # counter_results = await _retarget_subtitles_in_db_async(ref_phash_file_hash, ref_phash_file, ref_subtitles, target_phash_file_hash)
        # [CPU]
        counter_results = retarget_subtitles_with_bktree(ref_phash_file, ref_subtitles, target_phash_file)

    with _Timer("Shift subtitles (from reference) and save it (for target)", func_to_log=console.print):
        # TODO: need to work about framerates from reference and target
        try:
            target_frame_rate = float(next(re.finditer(framerate_r, str(target_phash_file)))["framerate"])
        except StopIteration:
            target_frame_rate = DEFAULT_FRAMERATE
        console.print(f"{target_frame_rate=} fps")
        try:
            shift_offset_frames: int = counter_results[0][0]
            shift_offset_milliseconds: float = (shift_offset_frames / target_frame_rate) * 1000
            console.print(
                f"Shift subtitles seconds = {shift_offset_frames} frames ~ {shift_offset_milliseconds / 1000:.4f} seconds"
            )
            subs = pysrt.open(ref_subtitles)
            subs.shift(milliseconds=shift_offset_milliseconds)
            # TODO: need to parametrize directory export
            subs_target_filepath = Path(gettempdir()) / target_phash_file.with_suffix(".srt").name
            subs.save(subs_target_filepath)
            console.print(f"Exported subtitles for target in : '{subs_target_filepath}'")
        except IndexError:
            console.print("Error: Can't find difference offsets !")
    return subs_target_filepath
