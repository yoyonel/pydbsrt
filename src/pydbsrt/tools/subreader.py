"""
"""
import codecs
from dataclasses import dataclass, field
from pathlib import Path

import pysrt


@dataclass
class SubReader:
    path: Path
    encoding: str = "utf-8"
    #
    file: codecs.StreamReaderWriter = field(init=False)
    stream: pysrt.SubRipFile = field(init=False)

    def __post_init__(self):
        self.file = codecs.open(str(self.path), encoding=self.encoding)
        self.stream = pysrt.stream(self.file)

    def __iter__(self):
        yield from self.stream
        return
