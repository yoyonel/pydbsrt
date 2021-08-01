from pathlib import Path
from typing import Coroutine

import pytest
from asyncpg import Connection
from waiting import wait

from pydbsrt.services.database import create_conn, drop_tables_async


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
