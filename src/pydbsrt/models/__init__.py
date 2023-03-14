from dataclasses import astuple


class UnpackIterable:
    # [Implement packing/unpacking in an object](https://stackoverflow.com/a/70753113)
    def __iter__(self):
        return iter(astuple(self))
