"""

"""
import attr
from pysrt.srtitem import SubRipTime
#
from pydbsrt.tools.videofingerprint import VideoFingerprint
from pydbsrt.tools.subreader import SubReader


def subriptime_to_frame(srt: SubRipTime) -> int:
    total_seconds = srt.hours * 60 * 60 + srt.minutes * 60 + srt.seconds + srt.milliseconds / 1000.0
    return int(total_seconds * 25)


@attr.s()
class SubFingerprints:
    """

    """
    subreader = attr.ib(type=SubReader)
    vfp = attr.ib(type=VideoFingerprint)

    def __iter__(self):
        # we suppose subtitles generator in order

        fp = next(self.vfp)
        id_frame = 0

        # iter on subtitles generator
        for subtitle in self.subreader:
            # get start, end timecodes
            tc_start, tc_end = subtitle.start, subtitle.end
            frame_start, frame_end = subriptime_to_frame(tc_start), subriptime_to_frame(tc_end)

            # print(frame_start, frame_end)
            try:
                while not(id_frame <= frame_start <= id_frame + 1):
                    # print(id_frame, frame_start, frame_end)
                    fp = next(self.vfp)
                    id_frame += 1

                yield subtitle.index, id_frame, fp

                while not (id_frame <= frame_end <= id_frame + 1):
                    fp = next(self.vfp)
                    id_frame += 1
                    yield subtitle.index, id_frame, fp

                yield subtitle.index, id_frame, fp

            except StopIteration:
                break
