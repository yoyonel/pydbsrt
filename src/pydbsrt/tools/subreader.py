"""

"""
import attr
import codecs
from pathlib import Path
import pysrt


@attr.s
class SubReader:
    path = attr.ib(type=Path)

    file = attr.ib(init=False, type=codecs.StreamReaderWriter)
    encoding = attr.ib(type=str, default='utf-8')

    stream = attr.ib(init=False, type=pysrt.SubRipFile)

    def __attrs_post_init__(self):
        self.file = codecs.open(str(self.path), encoding=self.encoding)
        self.stream = pysrt.stream(self.file)

    def __iter__(self):
        for sub in self.stream:
            yield sub
        return
