"""Moviehash of a NAS media read over FTP: same vectors as crates/dbsrt-core/tests/test_moviehash.rs."""

import ftplib
from typing import cast

import pytest

from scripts.o2switch_batch_worker import MOVIEHASH_CHUNK_BYTES, ftp_moviehash, moviehash_from_chunks

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

    def transfercmd(self, cmd: str, rest: int | None = None) -> FakeConn:
        self.commands.append((cmd, rest))
        conn = FakeConn(self.data[rest or 0 :])
        self.conns.append(conn)
        return conn

    def voidresp(self) -> str:
        # The server reports the transfer we closed early as aborted
        raise ftplib.error_temp("426 Connection closed; transfer aborted.")


@pytest.mark.parametrize(("size", "expected"), VECTORS)
def test_moviehash_from_chunks_matches_rust_vectors(size: int, expected: str) -> None:
    data = pattern(size)
    head, tail = data[:MOVIEHASH_CHUNK_BYTES], data[-MOVIEHASH_CHUNK_BYTES:]
    assert moviehash_from_chunks(size, head, tail) == expected


@pytest.mark.parametrize(("size", "expected"), VECTORS)
def test_ftp_moviehash_reads_head_and_tail_only(size: int, expected: str) -> None:
    ftp = FakeFtp(pattern(size))
    assert ftp_moviehash(cast(ftplib.FTP, ftp), "/video/Raw Name.mkv", size) == expected
    assert ftp.commands == [
        ("RETR /video/Raw Name.mkv", 0),
        ("RETR /video/Raw Name.mkv", size - MOVIEHASH_CHUNK_BYTES),
    ]
    assert all(conn.closed for conn in ftp.conns)


def test_ftp_moviehash_skips_files_under_one_chunk() -> None:
    ftp = FakeFtp(pattern(65_535))
    assert ftp_moviehash(cast(ftplib.FTP, ftp), "/video/x.mkv", 65_535) is None
    assert ftp.commands == []


def test_ftp_moviehash_short_read_gives_no_hash() -> None:
    # Listed size larger than what the server sends: a hash of the wrong bytes would name the wrong .phash
    ftp = FakeFtp(pattern(100_000))
    assert ftp_moviehash(cast(ftplib.FTP, ftp), "/video/x.mkv", 200_000) is None
