from more_itertools import grouper
from pysrt import InvalidItem
from pysrt.compat import is_py2
from pysrt.comparablemixin import ComparableMixin


class SubFingerPrintRipItem(ComparableMixin):
    """
    SubFingerPrintRipItem(srt_index, frame_index, fingerprints)

    srt_index -> int: joint key with subtitle. 0 by default.
    frame_index -> int: index of item in file. 0 by default.
    fingerprints -> list[int64]: Fingerprints (hash) content for item.
    """
    ITEM_PATTERN = str('%s\n%s\n%s\n')

    def __init__(self, srt_index=0, frame_index=0, fingerprint: int = 0x0):
        try:
            self.srt_index = int(srt_index)
        except (TypeError, ValueError):  # try to cast as int, but it's not mandatory
            self.srt_index = srt_index

        try:
            self.frame_index = int(frame_index)
        except (TypeError, ValueError):  # try to cast as int, but it's not mandatory
            self.frame_index = frame_index

        self.fingerprint = fingerprint

    def __str__(self):
        return self.ITEM_PATTERN % (self.srt_index, self.frame_index, str(self.fingerprint))

    def __hash__(self):
        return hash(str(self))

    if is_py2:
        __unicode__ = __str__

        def __str__(self):
            raise NotImplementedError('Use unicode() instead!')

    def _cmpkey(self):
        return (self.srt_index, self.frame_index)

    @classmethod
    def from_string(cls, source):
        return cls.from_lines(source.splitlines(True))

    @classmethod
    def from_lines(cls, lines):
        if len(lines) < 2:
            raise InvalidItem()
        lines = [line.rstrip() for line in lines]
        srt_index = lines.pop(0)
        frame_index = lines.pop(0)
        fingerprint = int(lines.pop(0), 16)
        return cls(srt_index, frame_index, fingerprint)
