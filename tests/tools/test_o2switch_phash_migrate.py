"""Migration of legacy <media file>.<fps>fps.phash names to <moviehash>.<fps>fps.phash plus sidecar."""

import json
import threading
from pathlib import Path

from scripts.o2switch_batch_worker import VideoTarget
from scripts.o2switch_phash_migrate import apply_moves, plan_migration

HASH_A = "e19d5212c9812cd6"


def write_phash(phash_dir: Path, name: str, frames: int = 3) -> Path:
    path = phash_dir / name
    path.write_bytes(b"\x01" * 8 * frames)
    return path


def nas(*names: str) -> list[VideoTarget]:
    return [VideoTarget(f"/video/{i}/{name}", name, 200_000) for i, name in enumerate(names)]


def test_plan_migrates_only_a_legacy_name_found_once_on_the_nas(tmp_path: Path) -> None:
    write_phash(tmp_path, "Movie A.mkv.25.0fps.phash")
    write_phash(tmp_path, f"{HASH_A}.23.976fps.phash")
    write_phash(tmp_path, "Orphan.mkv.25.0fps.phash")
    write_phash(tmp_path, "Dup.mkv.25.0fps.phash")
    write_phash(tmp_path, "NoFps.mkv.phash")
    write_phash(tmp_path, "Empty.mkv.25.0fps.phash", frames=0)
    hashed: list[str] = []

    def moviehash_of(video: VideoTarget) -> str:
        hashed.append(video.filename)
        return "0123456789abcdef"

    moves, counts = plan_migration(tmp_path, nas("Movie A.mkv", "Dup.mkv", "Dup.mkv"), moviehash_of)

    assert moves == [
        (tmp_path / "Movie A.mkv.25.0fps.phash", tmp_path / "0123456789abcdef.25.0fps.phash", "Movie A.mkv")
    ]
    assert counts == {
        "to_migrate": 1,
        "already_moviehash": 1,
        "not_on_nas": 1,
        "ambiguous_on_nas": 1,
        "no_fps_suffix": 1,
        "empty": 1,
    }
    # The NAS is read only for the files that can be migrated
    assert hashed == ["Movie A.mkv"]


def test_plan_matches_a_url_encoded_legacy_name(tmp_path: Path) -> None:
    write_phash(tmp_path, "Movie%20A.mkv.25.0fps.phash")
    moves, _ = plan_migration(tmp_path, nas("Movie A.mkv"), lambda video: HASH_A)
    assert [media_name for _, _, media_name in moves] == ["Movie A.mkv"]


def test_plan_never_overwrites_an_existing_moviehash_file(tmp_path: Path) -> None:
    write_phash(tmp_path, "Movie A.mkv.25.0fps.phash")
    write_phash(tmp_path, "Movie B.mkv.25.0fps.phash")
    write_phash(tmp_path, f"{HASH_A}.25.0fps.phash", frames=5)
    # Two legacy names of the same content, and a file already named by that moviehash
    moves, counts = plan_migration(tmp_path, nas("Movie A.mkv", "Movie B.mkv"), lambda video: HASH_A)
    assert moves == []
    assert counts["target_exists"] == 2


def test_plan_skips_a_media_whose_moviehash_cannot_be_read(tmp_path: Path) -> None:
    write_phash(tmp_path, "Movie A.mkv.25.0fps.phash")
    moves, counts = plan_migration(tmp_path, nas("Movie A.mkv"), lambda video: None)
    assert moves == []
    assert counts["no_moviehash"] == 1


def test_apply_renames_and_writes_the_sidecar_without_any_nas_location(tmp_path: Path) -> None:
    old = write_phash(tmp_path, "Movie A.mkv.25.0fps.phash", frames=3)
    new = tmp_path / f"{HASH_A}.25.0fps.phash"

    assert apply_moves([(old, new, "Movie A.mkv")]) == 1

    assert not old.exists() and new.read_bytes() == b"\x01" * 24
    sidecar = json.loads((tmp_path / f"{HASH_A}.json").read_text(encoding="utf-8"))
    assert sidecar["moviehash"] == HASH_A
    assert sidecar["media_name"] == "Movie A.mkv"
    assert sidecar["fps"] == 25.0
    assert sidecar["frames"] == 3
    assert "/video" not in json.dumps(sidecar)


def test_apply_keeps_an_existing_sidecar_and_an_existing_target(tmp_path: Path) -> None:
    old = write_phash(tmp_path, "Movie A.mkv.25.0fps.phash")
    sidecar = tmp_path / f"{HASH_A}.json"
    sidecar.write_text('{"media_name": "kept"}', encoding="utf-8")
    taken = write_phash(tmp_path, f"{HASH_A}.25.0fps.phash", frames=5)

    # The target appeared after planning: the legacy file stays, nothing is overwritten
    assert apply_moves([(old, taken, "Movie A.mkv")]) == 0

    assert old.exists() and taken.stat().st_size == 40
    assert json.loads(sidecar.read_text(encoding="utf-8")) == {"media_name": "kept"}


def test_plan_hashes_in_parallel_and_keeps_the_file_order(tmp_path: Path) -> None:
    names = [f"Movie {i:02d}.mkv" for i in range(12)]
    for name in names:
        write_phash(tmp_path, f"{name}.25.0fps.phash")
    gate = threading.Barrier(4, timeout=5)

    def moviehash_of(video: VideoTarget) -> str:
        # Four hashes must run at the same time, or the barrier times out
        gate.wait()
        return f"{int(video.filename[6:8]):016x}"

    moves, counts = plan_migration(tmp_path, nas(*names), moviehash_of, jobs=4)

    assert counts == {"to_migrate": 12}
    assert [media_name for _, _, media_name in moves] == names
