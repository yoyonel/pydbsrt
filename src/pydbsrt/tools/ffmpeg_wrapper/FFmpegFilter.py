"""

Class designed to handle FFmpeg filters

"""


class FFmpegFilter:
    """A simple builder a ffmpeg filter"""

    def __init__(self, name):
        self.name = name
        self.opts = dict()

    def set_option(self, name, value):
        """
        Set ffmpeg filter option.
        """
        self.opts[name] = value

    def remove_option(self, name):
        """
        Remove ffmpeg filter option.
        """
        try:
            self.opts[name]
        except KeyError:
            pass

    def build(self):
        r"""
        Build a valid ffmpeg filter string.

        Example for filter 'select' with option 'expr' set to 'eq(n\\, 0)' and option 'showinfo':
        "select=expr=eq(n\\,0),showinfo"
        """
        built_string = self.name
        if self.opts:
            built_string += "="
            built_string += ":".join(f"{opt}={val}" for opt, val in self.opts.items())
        return built_string
