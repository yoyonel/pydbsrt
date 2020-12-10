from pathlib import Path

import pytest


@pytest.fixture()
def resource_video_path():
    """
    https://stackoverflow.com/a/47704630 (How to generate a 2hour-long blank video)
    ffmpeg commands:
        - `ffmpeg -y -t 1 -f lavfi -i color=c=white:s=32x32 -c:v libx264 -tune stillimage -pix_fmt yuv420p white_frames.mp4`
        - `ffmpeg -y -t 1 -f lavfi -i color=c=black:s=32x32 -c:v libx264 -tune stillimage -pix_fmt yuv420p white_frames.mp4`
    TODO: tester d'autres encodages !
    """
    return lambda color_frame: Path(f"data/{color_frame}_frames.mp4")
