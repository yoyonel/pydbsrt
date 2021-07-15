from pathlib import Path

import pytest


@pytest.fixture()
def resource_video_path():
    """"""

    def _resource_video_path(video_filename: str):
        p_rvp = Path(f"data/{video_filename}")
        assert p_rvp.exists()
        return p_rvp

    return _resource_video_path
