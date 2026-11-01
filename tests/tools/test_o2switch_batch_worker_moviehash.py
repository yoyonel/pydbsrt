"""Moviehash of a NAS media read over FTP: same vectors as crates/dbsrt-core/tests/test_moviehash.rs."""

import ftplib
import os
import threading
from pathlib import Path
from typing import Callable, cast

import pytest

import scripts.o2switch_batch_worker as worker
from scripts.o2switch_batch_worker import (
    MOVIEHASH_CHUNK_BYTES,
    VideoTarget,
    ftp_moviehash,
    moviehash_from_chunks,
)

VECTORS = [
    (200_000, "e19d5212c9812cd6"),
    (100_000, "e59c5111c783a536"),
    (65_536, "d48b4000b76d0d86"),
]


def pattern(size: int) -> bytes:
    return bytes(i % 251 for i in range(size))


class FakeConn:
    """Data socket that hands out at most 8 KiB per recv, like a real network read."""

    def __init__(self, data: bytes) -> None:
        self.data = data
        self.closed = False

    def recv(self, n: int) -> bytes:
        chunk, self.data = self.data[: min(n, 8192)], self.data[min(n, 8192) :]
        return chunk

    def close(self) -> None:
        self.closed = True


class FakeFtp:
    def __init__(self, data: bytes) -> None:
        self.data = data
        self.commands: list[tuple[str, int | None]] = []
        self.conns: list[FakeConn] = []
        self.closed = False

    def close(self) -> None:
        self.closed = True

    def transfercmd(self, cmd: str, rest: int | None = None) -> FakeConn:
        self.commands.append((cmd, rest))
        conn = FakeConn(self.data[rest or 0 :])
        self.conns.append(conn)
        return conn

    def voidresp(self) -> str:
        # The server reports the transfer we closed early as aborted
        raise ftplib.error_temp("426 Connection closed; transfer aborted.")


class DroppingFtp(FakeFtp):
    """Measured on the Synology NAS: an early-closed RETR gets no 426, the control connection is dropped."""

    def __init__(self, data: bytes) -> None:
        super().__init__(data)
        self.dropped = False

    def transfercmd(self, cmd: str, rest: int | None = None) -> FakeConn:
        if self.dropped:
            raise EOFError
        return super().transfercmd(cmd, rest)

    def voidresp(self) -> str:
        self.dropped = True
        raise EOFError


def sessions(cls: type[FakeFtp], data: bytes) -> tuple[list[FakeFtp], Callable[[], ftplib.FTP]]:
    """FTP session factory that records every session it opens."""
    opened: list[FakeFtp] = []

    def connect() -> ftplib.FTP:
        opened.append(cls(data))
        return cast(ftplib.FTP, opened[-1])

    return opened, connect


@pytest.mark.parametrize(("size", "expected"), VECTORS)
def test_moviehash_from_chunks_matches_rust_vectors(size: int, expected: str) -> None:
    data = pattern(size)
    head, tail = data[:MOVIEHASH_CHUNK_BYTES], data[-MOVIEHASH_CHUNK_BYTES:]
    assert moviehash_from_chunks(size, head, tail) == expected


@pytest.mark.parametrize(("size", "expected"), VECTORS)
def test_ftp_moviehash_reads_head_and_tail_only(size: int, expected: str) -> None:
    opened, connect = sessions(FakeFtp, pattern(size))
    assert ftp_moviehash(connect, "/video/Raw Name.mkv", size) == expected
    assert [ftp.commands for ftp in opened] == [
        [("RETR /video/Raw Name.mkv", 0)],
        [("RETR /video/Raw Name.mkv", size - MOVIEHASH_CHUNK_BYTES)],
    ]
    assert all(ftp.closed and all(conn.closed for conn in ftp.conns) for ftp in opened)


def test_ftp_moviehash_survives_a_nas_that_drops_the_control_connection() -> None:
    opened, connect = sessions(DroppingFtp, pattern(200_000))
    assert ftp_moviehash(connect, "/video/x.mkv", 200_000) == "e19d5212c9812cd6"
    assert len(opened) == 2 and all(ftp.closed for ftp in opened)


def test_ftp_moviehash_skips_files_under_one_chunk() -> None:
    opened, connect = sessions(FakeFtp, pattern(65_535))
    assert ftp_moviehash(connect, "/video/x.mkv", 65_535) is None
    assert opened == []


def test_ftp_moviehash_short_read_gives_no_hash() -> None:
    # Listed size larger than what the server sends: a hash of the wrong bytes would name the wrong .phash
    _, connect = sessions(FakeFtp, pattern(100_000))
    assert ftp_moviehash(connect, "/video/x.mkv", 200_000) is None


def _run_video(tmp_path: Path, monkeypatch: pytest.MonkeyPatch, moviehash: str | None) -> list[str]:
    """Runs process_single_video with a fake dbsrt that prints its argv, one arg per line."""
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(worker, "target_moviehash", lambda target, stream_base, env: moviehash)
    fake_dbsrt = tmp_path / "dbsrt"
    fake_dbsrt.write_text('#!/bin/sh\nprintf "%s\\n" "$@"\n', encoding="utf-8")
    fake_dbsrt.chmod(0o755)
    target = VideoTarget("/tv/Raw Name.mkv", "Raw Name.mkv", 200_000)
    env = {"PATH": os.environ["PATH"]}
    worker.process_single_video(target, "ftp://nas:21", str(fake_dbsrt), env, threading.Lock(), tmp_path / "ev.jsonl")
    return sorted(p.name for p in (tmp_path / "logs" / "workers").iterdir())


def test_process_single_video_names_the_log_by_moviehash(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    # A 250+ byte media name cannot name a file (os error 36): the moviehash can
    assert _run_video(tmp_path, monkeypatch, "e19d5212c9812cd6") == ["e19d5212c9812cd6.log"]
    argv = (tmp_path / "logs" / "workers" / "e19d5212c9812cd6.log").read_text(encoding="utf-8").splitlines()
    assert argv[argv.index("--moviehash") + 1] == "e19d5212c9812cd6"


def test_process_single_video_without_moviehash_keeps_the_stem(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    assert _run_video(tmp_path, monkeypatch, None) == ["Raw Name.log"]
    assert "--moviehash" not in (tmp_path / "logs" / "workers" / "Raw Name.log").read_text(encoding="utf-8")


def test_target_moviehash_is_none_when_the_nas_is_unreachable() -> None:
    target = VideoTarget("/tv/a.mkv", "a.mkv", 200_000)
    # Port 1 on localhost refuses the connection: no hash, the worker falls back to the stem
    assert worker.target_moviehash(target, "ftp://127.0.0.1:1", {}) is None
    assert worker.target_moviehash(target, "http://127.0.0.1:1", {}) is None
