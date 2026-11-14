"""Moviehash of a NAS media read over FTP: same vectors as crates/dbsrt-core/tests/test_moviehash.rs."""

import ftplib
import os
import threading
from http.server import BaseHTTPRequestHandler, SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Callable, Iterator, cast

import pytest

import scripts.workers.o2switch_batch_worker as worker
from scripts.infra.rpi4.rpi4_freebox_gateway import FreeboxRangeHTTPHandler
from scripts.workers.o2switch_batch_worker import (
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
    def __init__(
        self,
        data: bytes,
        paths: dict[str, bytes] | None = None,
        reported_size: int | None = None,
    ) -> None:
        self.data = data
        self.paths = paths
        self.reported_size = reported_size
        self.commands: list[tuple[str, int | None]] = []
        self.conns: list[FakeConn] = []
        self.closed = False
        self.type_cmds: list[str] = []

    def close(self) -> None:
        self.closed = True

    def voidcmd(self, cmd: str) -> str:
        self.type_cmds.append(cmd)
        return "200 Type set to I."

    def size(self, path: str) -> int:
        if self.reported_size is not None:
            return self.reported_size
        if self.paths is not None:
            if path not in self.paths:
                raise ftplib.error_perm("550 No such file.")
            return len(self.paths[path])
        return len(self.data)

    def transfercmd(self, cmd: str, rest: int | None = None) -> FakeConn:
        self.commands.append((cmd, rest))
        payload = self.data
        if self.paths is not None:
            req_path = cmd.removeprefix("RETR ")
            payload = self.paths.get(req_path, b"")
        conn = FakeConn(payload[rest or 0 :])
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
        [],
        [("RETR /video/Raw Name.mkv", 0)],
        [("RETR /video/Raw Name.mkv", size - MOVIEHASH_CHUNK_BYTES)],
    ]
    assert all(f.type_cmds == ["TYPE I"] for f in opened)
    assert all(ftp.closed and all(conn.closed for conn in ftp.conns) for ftp in opened)


def test_ftp_moviehash_survives_a_nas_that_drops_the_control_connection() -> None:
    opened, connect = sessions(DroppingFtp, pattern(200_000))
    assert ftp_moviehash(connect, "/video/x.mkv", 200_000) == "e19d5212c9812cd6"
    assert len(opened) == 3 and all(ftp.closed for ftp in opened)
    assert all(f.type_cmds == ["TYPE I"] for f in opened)


def test_ftp_moviehash_skips_files_under_one_chunk() -> None:
    opened, connect = sessions(FakeFtp, pattern(65_535))
    assert ftp_moviehash(connect, "/video/x.mkv", 65_535) is None
    assert len(opened) == 1 and opened[0].commands == []
    assert all(f.type_cmds == ["TYPE I"] for f in opened)


def test_ftp_moviehash_short_read_gives_no_hash() -> None:
    # Server SIZE reports 200_000 but RETR stream truncates at 100_000: must reject rather than hash wrong bytes
    opened: list[FakeFtp] = []

    def connect() -> ftplib.FTP:
        opened.append(FakeFtp(pattern(100_000), reported_size=200_000))
        return cast(ftplib.FTP, opened[-1])

    assert ftp_moviehash(connect, "/video/x.mkv", 200_000) is None
    assert all(f.type_cmds == ["TYPE I"] for f in opened)


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


def test_process_single_video_truncates_and_hashes_overlong_stem_without_moviehash(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.chdir(tmp_path)
    monkeypatch.setattr(worker, "target_moviehash", lambda target, stream_base, env: None)
    fake_dbsrt = tmp_path / "dbsrt"
    fake_dbsrt.write_text('#!/bin/sh\nprintf "%s\\n" "$@"\n', encoding="utf-8")
    fake_dbsrt.chmod(0o755)
    overlong_name = ("Épisode_Très_Long_" * 20) + ".mkv"
    target = VideoTarget(f"/tv/{overlong_name}", overlong_name, 200_000)
    env = {"PATH": os.environ["PATH"]}
    rec = worker.process_single_video(
        target, "ftp://nas:21", str(fake_dbsrt), env, threading.Lock(), tmp_path / "ev.jsonl"
    )
    assert rec["success"] is True
    logs = sorted(p.name for p in (tmp_path / "logs" / "workers").iterdir())
    assert len(logs) == 1
    assert len(logs[0].encode("utf-8")) < 200


FREEBOX_TARGET = VideoTarget("/Disque_1_1/Vidéos/Raw Name.mkv", "Raw Name.mkv", 200_000)


def _serve(handler: type[BaseHTTPRequestHandler], root: Path) -> Iterator[str]:
    media = root / "Disque_1_1" / "Vidéos" / "Raw Name.mkv"
    media.parent.mkdir(parents=True)
    media.write_bytes(pattern(200_000))
    server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        yield f"http://127.0.0.1:{server.server_port}"
    finally:
        server.shutdown()
        server.server_close()


@pytest.fixture
def freebox_gateway(tmp_path: Path) -> Iterator[str]:
    """The real RPi4 Freebox gateway handler, serving one media from a temp library."""
    handler = type("Gateway", (FreeboxRangeHTTPHandler,), {"server_root": tmp_path, "log_message": lambda *_: None})
    yield from _serve(handler, tmp_path)


@pytest.fixture
def rangeless_server(tmp_path: Path) -> Iterator[str]:
    """A server that ignores Range and always sends the whole file (200)."""
    handler = type(
        "Rangeless",
        (SimpleHTTPRequestHandler,),
        {"__init__": lambda self, *a, **kw: SimpleHTTPRequestHandler.__init__(self, *a, directory=str(tmp_path), **kw)},
    )
    handler.log_message = lambda *_: None  # type: ignore[method-assign]
    yield from _serve(handler, tmp_path)


def test_target_moviehash_reads_head_and_tail_through_the_freebox_gateway(freebox_gateway: str) -> None:
    assert worker.target_moviehash(FREEBOX_TARGET, freebox_gateway, {}) == "e19d5212c9812cd6"


def test_target_moviehash_over_http_is_none_when_the_listed_size_is_wrong(freebox_gateway: str) -> None:
    # Both oversized and undersized stale inventory sizes must be rejected via Content-Range /total validation
    assert worker.target_moviehash(FREEBOX_TARGET._replace(size_bytes=300_000), freebox_gateway, {}) is None
    assert worker.target_moviehash(FREEBOX_TARGET._replace(size_bytes=150_000), freebox_gateway, {}) is None


def test_target_moviehash_over_http_is_none_when_the_server_ignores_range(rangeless_server: str) -> None:
    # A 200 full body would hash the head twice
    assert worker.target_moviehash(FREEBOX_TARGET, rangeless_server, {}) is None


def test_target_moviehash_is_none_when_the_nas_is_unreachable() -> None:
    target = VideoTarget("/tv/a.mkv", "a.mkv", 200_000)
    # Port 1 on localhost refuses the connection: no hash, the worker falls back to the stem
    assert worker.target_moviehash(target, "ftp://127.0.0.1:1", {}) is None
    assert worker.target_moviehash(target, "http://127.0.0.1:1", {}) is None


def test_ftp_moviehash_probes_size_when_zero_and_handles_nfd() -> None:
    import unicodedata

    data = pattern(200_000)
    nfd_path = unicodedata.normalize("NFD", "/tvshow/Café/épisode.mkv")
    nfc_path = unicodedata.normalize("NFC", "/tvshow/Café/épisode.mkv")
    opened: list[FakeFtp] = []

    def connect() -> ftplib.FTP:
        opened.append(FakeFtp(data, paths={nfd_path: data}))
        return cast(ftplib.FTP, opened[-1])

    assert ftp_moviehash(connect, nfc_path, 0) == "e19d5212c9812cd6"
    assert all(f.type_cmds == ["TYPE I"] for f in opened)
    assert [ftp.commands for ftp in opened[1:]] == [
        [(f"RETR {nfd_path}", 0)],
        [(f"RETR {nfd_path}", 200_000 - MOVIEHASH_CHUNK_BYTES)],
    ]

    # Even when a non-zero stale size is passed, authoritative FTP SIZE + NFD fallback win
    opened.clear()
    assert ftp_moviehash(connect, nfc_path, 999_999) == "e19d5212c9812cd6"
    assert all(f.type_cmds == ["TYPE I"] for f in opened)
    assert [ftp.commands for ftp in opened[1:]] == [
        [(f"RETR {nfd_path}", 0)],
        [(f"RETR {nfd_path}", 200_000 - MOVIEHASH_CHUNK_BYTES)],
    ]


def test_strip_fps_token_preserves_years_and_episode_numbers() -> None:
    assert worker.ahash_logical_stem("Blade.Runner.2049.24fps.ahash") == "Blade.Runner.2049"
    assert worker.ahash_logical_stem("Blade.Runner.2049.86.1fps.ahash") == "Blade.Runner.2049"
    assert worker.ahash_logical_stem("Show.S01.02.24fps.ahash") == "Show.S01.02"
    assert worker.phash_media_stem("Blade.Runner.2049.mkv.23.976fps.phash") == "Blade.Runner.2049"


def test_target_moviehash_over_http_probes_size_when_zero(freebox_gateway: str) -> None:
    target = FREEBOX_TARGET._replace(size_bytes=0)
    assert worker.target_moviehash(target, freebox_gateway, {}) == "e19d5212c9812cd6"


@pytest.mark.parametrize(("size", "expected"), VECTORS)
def test_compute_local_moviehash_matches_vectors(tmp_path: Path, size: int, expected: str) -> None:
    p = tmp_path / "sample.mkv"
    p.write_bytes(pattern(size))
    assert worker.compute_local_moviehash(p) == expected
    small = tmp_path / "small.mkv"
    small.write_bytes(pattern(MOVIEHASH_CHUNK_BYTES - 1))
    assert worker.compute_local_moviehash(small) is None
