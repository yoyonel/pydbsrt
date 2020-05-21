from itertools import chain

from pysrt import ERROR_PASS
from pysrt.srtexc import Error
from pysrt.srtfile import SubRipFile

from pydbsrt.tools.srt_fp_item import SubFingerPrintRipItem


class SubFingerPrintRipFile(SubRipFile):
    def slice(self, *_args, **_kwargs):
        raise NotImplementedError

    def at(self, *_args, **_kwargs):
        raise NotImplementedError

    def clean_indexes(self):
        raise NotImplementedError

    @property
    def text(self):
        raise NotImplementedError

    @classmethod
    def stream(cls, source_file, error_handling=ERROR_PASS):
        """ """
        string_buffer = []
        for index, line in enumerate(chain(source_file, '\n')):
            if line.strip():
                string_buffer.append(line)
            else:
                source = string_buffer
                string_buffer = []
                if source and all(source):
                    try:
                        yield SubFingerPrintRipItem.from_lines(source)
                    except Error as error:
                        error.args += (''.join(source),)
                        cls._handle_error(error, error_handling, index)
