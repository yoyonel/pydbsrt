[
    search_tree.map_node_value_to_data[node_hash]
    for node_hash in search_tree.tree.getWithinDistance(searched_node_hash, search_dist)
]

from collections import Counter

{
    srt_uuid: Counter(srt_indexes)
    for srt_uuid, srt_indexes in matching_results.items()
}


c15d3cbf88c16dd2ce38e28292c1f9e8

# extract subtitles from media
media_filename:
- Silicon.Valley.S04E01.Success.Failure.1080p.AMZN.WEB-DL.DD.5.1.H.265-SiGMA

ffmpeg -i %{media_filename}.mkv -map 0:2 -c:s srt %{media_filename}.en.srt

# extract 5s cut from media
ffmpeg \
	-y -hide_banner \
	-ss 0:00:00 \
	-i %{media_filename} \
	-vframes 1440 \
	-c copy \
	~/tmp/%{media_filename}.cut.mkv

# rescale 32x32 - gray format - remove audio
https://lists.ffmpeg.org/pipermail/ffmpeg-user/2012-August/008554.html

ffmpeg \
	-y -hide_banner \
	-ss 0:00:00 \
	-i $INPUT_MEDIA \
	-vframes 1440 \
	-an \
	-vf "scale=32:32,format=gray" \
	$INPUT_MEDIA.cut.mkv

# cut: full size, reduce (32x32) in 8 bits (gray)

## full
search img hash: 125 image hash [00:00, 154.61 image hash/s]
2020-05-25 16:39:03,429 - __main__ - INFO - Fingerprinting request search media => took 0.8145 seconds
2020-05-25 16:39:03,445 - __main__ - INFO - Search fingerprint in Search Tree => took 0.0155 seconds

## reduce
search img hash: 120 image hash [00:00, 1149.07 image hash/s]
2020-05-25 16:37:50,326 - __main__ - INFO - Fingerprinting request search media => took 0.1056 seconds
2020-05-25 16:37:50,340 - __main__ - INFO - Search fingerprint in Search Tree => took 0.0133 seconds
