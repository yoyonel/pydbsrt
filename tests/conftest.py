from pathlib import Path
from typing import Coroutine

import pytest
from asyncpg import Connection
from waiting import wait

from pydbsrt.services import export_imghash_from_media
from pydbsrt.services.database import create_conn, drop_tables_async
from pydbsrt.services.db_frames import import_binary_img_hash_to_db_async


@pytest.fixture()
def resource_video_path():
    """"""

    def _resource_video_path(video_filename: str):
        p_rvp = Path(f"{Path(__file__).parent}/data/{video_filename}")
        assert p_rvp.exists()
        return p_rvp

    return _resource_video_path


@pytest.fixture()
def resource_phash_path():
    """"""

    def _resource_phash_path(phash_filename: str):
        path_to_resource = Path(f"{Path(__file__).parent}/data/{phash_filename}").resolve()
        assert path_to_resource.exists()
        return path_to_resource

    return _resource_phash_path


@pytest.fixture(scope="session")
def db_is_ready(session_scoped_container_getter):
    # get database service
    bktreedb_service = next(
        service for service in session_scoped_container_getter.docker_project.services if service.name == "bktreedb"
    )
    # wait for a healthy status on database service
    wait(bktreedb_service.is_healthy)
    yield


@pytest.fixture(scope="function")
@pytest.mark.asyncio
async def conn(db_is_ready) -> Connection:
    conn = await create_conn()
    await drop_tables_async(conn, tables_names=("medias", "frames", "subtitles"))
    yield conn
    await drop_tables_async(conn, tables_names=("medias", "frames", "subtitles"))


@pytest.fixture(autouse=True)
def patch_coroclick(mocker, event_loop):
    def mocked_run_coro(coro: Coroutine):
        loop = event_loop
        task = loop.create_task(coro)
        loop.run_until_complete(task)

    mocker.patch("pydbsrt.tools.coro.run_coro", mocked_run_coro)


@pytest.fixture
async def phash_from_media(resource_video_path, tmpdir) -> Path:
    p_video = resource_video_path("big_buck_bunny_trailer_480p.webm")
    output_file_path = tmpdir.mkdir("phash") / f"{p_video.stem}.phash"
    output_file_exported: Path = export_imghash_from_media(p_video, output_file_path)
    return output_file_exported


@pytest.fixture
@pytest.mark.asyncio
async def aio_insert_phash_into_db(conn, phash_from_media: Path):
    media_id, nb_frames_inserted = await import_binary_img_hash_to_db_async(phash_from_media)
    assert media_id == 1
    assert nb_frames_inserted == 812
