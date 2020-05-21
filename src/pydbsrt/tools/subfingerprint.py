"""
"""
from dataclasses import dataclass
from pysrt.srtitem import SubRipTime
#
from pydbsrt.tools.videofingerprint import VideoFingerprint
from pydbsrt.tools.subreader import SubReader


def subriptime_to_frame(srt: SubRipTime) -> int:
    total_seconds = srt.hours * 60 * 60 + srt.minutes * 60 + srt.seconds + srt.milliseconds / 1000.0
    return int(total_seconds * 25)


@dataclass(init=True, repr=False, eq=False, order=False, unsafe_hash=False,
           frozen=True)
class SubFingerprints:
    """
"""
    sub_reader: SubReader
    video_fingerprint: VideoFingerprint

    def __iter__(self):
        # we suppose subtitles generator in order

        fingerprint = next(self.video_fingerprint)
        id_frame = 0

        # iter on subtitles generator
        for subtitle in self.sub_reader:
            # get start, end time codes
            frame_start, frame_end = subriptime_to_frame(subtitle.start), subriptime_to_frame(subtitle.end)
            try:
                # go through the first frame in subtitle interval time
                while not (id_frame <= frame_start <= id_frame + 1):
                    fingerprint = next(self.video_fingerprint)
                    id_frame += 1
                # yield the first couple subtitle, hash frame
                yield subtitle.index, id_frame, fingerprint
                # yield frame information until the frame is in subtitle interval time
                while not (id_frame <= frame_end <= id_frame + 1):
                    fingerprint = next(self.video_fingerprint)
                    id_frame += 1
                    yield subtitle.index, id_frame, fingerprint
                # yield the last couple subtitle, hash frame
                yield subtitle.index, id_frame, fingerprint
            except StopIteration:
                break
