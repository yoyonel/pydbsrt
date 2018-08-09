class BinaryNotFound(Exception):
    def __init__(self, binary_name):
        Exception.__init__(self)
        self.binary_name = binary_name


class BinaryCallFailed(Exception):
    def __init__(self):
        Exception.__init__(self)


class InvalidMedia(Exception):
    def __init__(self, media_path):
        Exception.__init__(self)
        self.media_path = media_path


class StreamIndexOutOfBound(Exception):
    def __init__(self):
        Exception.__init__(self)


class StreamEntryNotFound(Exception):
    def __init__(self):
        Exception.__init__(self)


class StreamLoadError(Exception):
    def __init__(self):
        Exception.__init__(self)