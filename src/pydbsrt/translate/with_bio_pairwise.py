#!/usr/bin/env python
import logging
from collections import defaultdict
from datetime import time, timedelta
from pathlib import Path
from pprint import PrettyPrinter
from typing import List, Tuple

import numpy as np
import pysrt
from pysrt import SubRipTime
from sklearn.cluster import MeanShift, estimate_bandwidth

from pydbsrt.translate.sync_srt import grouper_without_fill

logger = logging.getLogger(__name__)

medias_root_path = Path("data/")
pp = PrettyPrinter(indent=4)


def main():
    srt_from_path = medias_root_path / "The Golden Child (1986) 1080p web.en.srt"
    srt_from = pysrt.open(srt_from_path)
    srt_from_durations = [srt.duration for srt in srt_from]
    logger.info(f"srt_from_durations={pp.pformat(srt_from_durations)}")

    srt_to_path = medias_root_path / "The Golden Child.DVD_PAL.fre.srt"
    srt_to = pysrt.open(srt_to_path)
    srt_to_durations = [srt.duration for srt in srt_to]
    logger.info(f"srt_to_durations={pp.pformat(srt_to_durations)}")

    def _time_to_timedelta(t: time) -> timedelta:
        return timedelta(
            hours=t.hour,
            minutes=t.minute,
            seconds=t.second,
            milliseconds=t.microsecond / 1000,
        )

    def _clustering_srt_by_durations(
        durations: List[SubRipTime],
        bandwidth=None,
        quantile=0.50,
    ) -> Tuple[List[int], float]:
        x = [
            _time_to_timedelta(duration.to_time()).total_seconds()
            for duration in durations
        ]
        X = np.array(list(zip(x, np.zeros(len(x)))), dtype=float)
        if bandwidth is None:
            bandwidth = estimate_bandwidth(X, quantile=quantile)
        logger.info(f"bandwidth: {bandwidth}")
        ms = MeanShift(bandwidth=bandwidth, bin_seeding=True)
        ms.fit(X)
        labels = ms.labels_
        logger.info(f"nb unique labels: {len(np.unique(labels))}")
        # cluster_centers = ms.cluster_centers_
        # logger.info(f"cluster_centers: {cluster_centers}")
        # labels_unique = np.unique(labels)
        # n_clusters_ = len(labels_unique)
        # for k in range(n_clusters_):
        #     my_members = labels == k
        #     print("cluster {0}: {1}".format(k, X[my_members, 0]))
        return labels, bandwidth

    acgt = defaultdict(lambda: "*", ((0, "A"), (1, "C"), (2, "G"), (3, "T")))

    srt_from_labels, srt_from_bandwith = _clustering_srt_by_durations(
        srt_from_durations, quantile=0.20
    )
    srt_from_sequence_acgt = "".join([acgt[label] for label in srt_from_labels])
    s = "\n".join("".join(g) for g in grouper_without_fill(srt_from_sequence_acgt, 140))
    logger.info(f"srt_from_sequence_acgt:\n{s}")

    srt_to_labels, _ = _clustering_srt_by_durations(srt_to_durations, quantile=0.40)
    srt_to_sequence_acgt = "".join([acgt[label] for label in srt_to_labels])
    s = "\n".join("".join(g) for g in grouper_without_fill(srt_to_sequence_acgt, 140))
    logger.info(f"srt_to_sequence_acgt:\n{s}")


if __name__ == "__main__":
    main()
