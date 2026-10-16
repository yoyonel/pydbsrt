import pytest

from pydbsrt.tools.ffmpeg_tools.ffmpeg_cut import (
    build_parser,
    calculate_frame_pts_time,
    set_aselect_expression,
    set_select_expression,
)
from pydbsrt.tools.ffmpeg_wrapper import FFmpeg, FFmpegFilter, FFprobe


@pytest.fixture(autouse=True)
def setup_ffmpeg_binaries():
    try:
        FFmpeg.initialize()
        FFprobe.initialize()
    except Exception:
        pass


def test_ffmpeg_filter_basic():
    filt = FFmpegFilter.FFmpegFilter("select")
    assert filt.build() == "select"

    filt.set_option("expr", r"gte(n\,0)")
    assert filt.build() == r"select=expr=gte(n\,0)"

    filt.set_option("extra", "1")
    assert "extra=1" in filt.build()

    filt.remove_option("extra")
    assert filt.build() == r"select=expr=gte(n\,0)"

    # Removing non-existent option should not raise
    filt.remove_option("non_existent")


def test_set_select_expression():
    # Normal cases
    assert set_select_expression("BEGIN", "END") == r"gte(n\,0)"
    assert set_select_expression("BEGIN", 500) == r"gte(n\,0)*lte(n\,500)"
    assert set_select_expression(50, 500) == r"gte(n\,50)*lte(n\,500)"
    assert set_select_expression("100", "200") == r"gte(n\,100)*lte(n\,200)"

    # Invalid frame_start
    with pytest.raises(ValueError, match="Invalid start frame"):
        set_select_expression(-5, 100)

    with pytest.raises(ValueError):
        set_select_expression("not_an_int", 100)

    # Invalid frame_stop
    with pytest.raises(ValueError, match="Invalid stop frame"):
        set_select_expression(10, -10)

    with pytest.raises(ValueError):
        set_select_expression(10, "not_an_int")


def test_set_aselect_expression(big_buck_bunny_trailer):
    # BEGIN case
    assert set_aselect_expression("BEGIN", "END", str(big_buck_bunny_trailer)) == r"gte(t\,0)"

    # Valid media timestamp calculation
    res = set_aselect_expression(25, 50, str(big_buck_bunny_trailer))
    assert r"gte(t\," in res
    assert r"*lte(t\," in res

    # Invalid start
    with pytest.raises(ValueError, match="Invalid start frame"):
        set_aselect_expression(-1, "END", str(big_buck_bunny_trailer))

    # Invalid stop
    with pytest.raises(ValueError, match="Invalid stop frame"):
        set_aselect_expression(10, -5, str(big_buck_bunny_trailer))


def test_calculate_frame_pts_time(big_buck_bunny_trailer):
    pts = calculate_frame_pts_time(25, str(big_buck_bunny_trailer))
    assert pts is not None
    assert round(pts, 2) == 1.0  # 25 fps => frame 25 is at 1.0s

    # Invalid media path
    invalid_pts = calculate_frame_pts_time(10, "/tmp/non_existent_video_path.mp4")
    assert invalid_pts is None


def test_build_parser():
    parser = build_parser()
    assert parser is not None
    args = parser.parse_args(["-i", "in.mp4", "-o", "out.mp4", "-fstart", "10", "-fstop", "20"])
    assert args.input_file == "in.mp4"
    assert args.output_file == "out.mp4"
    assert args.frame_start == "10"
    assert args.frame_stop == "20"


def test_cut(big_buck_bunny_trailer, tmp_path):
    from pydbsrt.tools.ffmpeg_tools.ffmpeg_cut import cut

    out_file = tmp_path / "cut_output.mp4"
    ret = cut(
        input_file=str(big_buck_bunny_trailer),
        output_file=str(out_file),
        frame_start="0",
        frame_stop="5",
    )
    assert ret == 0
    assert out_file.exists()
