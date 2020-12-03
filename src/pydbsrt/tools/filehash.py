import hashlib
import io
from pathlib import Path


def md5_file(file_path: Path, block_size=io.DEFAULT_BUFFER_SIZE) -> str:
    with file_path.open("rb") as f:
        file_hash = hashlib.md5()
        # Python 3.8 style
        while chunk := f.read(block_size):
            file_hash.update(chunk)
    return file_hash.hexdigest()
