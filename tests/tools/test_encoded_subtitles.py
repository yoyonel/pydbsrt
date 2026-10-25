from pathlib import Path
from unittest.mock import MagicMock, patch

import pytest

from pydbsrt.tools.encoded_subtitles import (
    ExceptionExtractSubtitlesStreams,
    SubtitleStream,
    export_encoded_subtitles,
    extract_subtitles_streams,
)


def test_extract_subtitles_streams_no_embedded_subs():
    media_path = Path("tests/data/big_buck_bunny_trailer_480p.mp4")
    assert media_path.exists()
    streams = extract_subtitles_streams(media_path)
    assert isinstance(streams, list)
    assert len(streams) == 0


def test_export_encoded_subtitles_empty_when_no_subs(tmp_path):
    media_path = Path("tests/data/big_buck_bunny_trailer_480p.mp4")
    exported = export_encoded_subtitles(media_path, root_dir_export_subtitles_path=tmp_path)
    assert exported == []


def test_extract_subtitles_streams_error():
    with pytest.raises(ExceptionExtractSubtitlesStreams):
        extract_subtitles_streams(Path("/nonexistent/video.mkv"))


@patch("pydbsrt.tools.encoded_subtitles.sh.ffprobe")
def test_extract_subtitles_streams_mocked(mock_ffprobe):
    def fake_ffprobe(*args, **kwargs):
        callback = kwargs.get("_out")
        if callback:
            callback("2,fra\n")
            callback("3\n")
        mock_proc = MagicMock()
        mock_proc.wait.return_value = 0
        return mock_proc

    mock_ffprobe.side_effect = fake_ffprobe
    streams = extract_subtitles_streams(Path("fake_video.mp4"))
    assert len(streams) == 2
    assert streams[0] == SubtitleStream(2, "fra")
    assert streams[1] == SubtitleStream(3, "unk")


@patch("pydbsrt.tools.encoded_subtitles.sh.ffmpeg")
@patch("pydbsrt.tools.encoded_subtitles.extract_subtitles_streams")
def test_export_encoded_subtitles_mocked(mock_extract, mock_ffmpeg, tmp_path):
    mock_extract.return_value = [SubtitleStream(2, "fra"), SubtitleStream(3, "eng")]
    mock_proc = MagicMock()
    mock_proc.wait.return_value = 0
    mock_ffmpeg.return_value = mock_proc

    exported = export_encoded_subtitles(Path("video.mkv"), root_dir_export_subtitles_path=tmp_path)
    assert len(exported) == 2
    assert exported[0] == tmp_path / "video.mkv_fra_2.srt"
    assert exported[1] == tmp_path / "video.mkv_eng_3.srt"
    assert mock_ffmpeg.called
