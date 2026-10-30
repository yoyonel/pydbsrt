"""Unit tests for Freebox media scanner and exclusion filters."""

from pathlib import Path

from scripts.scan_freebox_targets import (
    _extract_target_from_file,
    is_path_excluded,
    scan_freebox_targets,
)


def test_is_path_excluded_personal_media():
    assert is_path_excluded(("Disque_1_1", "Photos", "Camera")) is True
    assert is_path_excluded(("Disque_1_1", "Backup", "WhatsApp")) is True
    assert is_path_excluded(("Disque_1", "Enregistrements")) is True
    assert is_path_excluded(("Disque_1", ".cameras")) is True
    assert is_path_excluded(("Disque_1", "lost+found")) is True


def test_is_path_excluded_whitelisted_media():
    assert is_path_excluded(("Disque_1", "Téléchargements", "__FRANCAIS__")) is False
    assert is_path_excluded(("Disque_2", "Téléchargements", "Séries")) is False
    assert is_path_excluded(("Disque_1_1", "Vidéos", "Films")) is False


def test_extract_target_from_file_valid(tmp_path: Path):
    root = tmp_path / "freebox"
    video = root / "Disque_1/Téléchargements/Film (2020).mkv"
    video.parent.mkdir(parents=True)
    video.write_bytes(b"dummy video content")

    target = _extract_target_from_file(video, root)
    assert target is not None
    assert target["filename"] == "Film (2020).mkv"
    assert target["stem"] == "Film (2020)"
    assert target["size_bytes"] == len(b"dummy video content")
    assert target["path"] == "Disque_1/Téléchargements/Film (2020).mkv"


def test_extract_target_from_file_unsupported_ext(tmp_path: Path):
    root = tmp_path / "freebox"
    nfo = root / "Disque_1/Téléchargements/Film (2020).nfo"
    nfo.parent.mkdir(parents=True)
    nfo.write_bytes(b"metadata")

    assert _extract_target_from_file(nfo, root) is None


def test_scan_freebox_targets_end_to_end(tmp_path: Path):
    root = tmp_path / "freebox"
    # Whitelisted movie
    movie = root / "Disque_1/Téléchargements/Movie.mp4"
    movie.parent.mkdir(parents=True)
    movie.write_bytes(b"movie data")

    # Blacklisted photos (even if video extension)
    photo_video = root / "Disque_1_1/Photos/VID_WA.mp4"
    photo_video.parent.mkdir(parents=True)
    photo_video.write_bytes(b"photo data")

    targets = scan_freebox_targets(root)
    assert len(targets) == 1
    assert targets[0]["filename"] == "Movie.mp4"
