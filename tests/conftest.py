from pathlib import Path

import pytest
from asyncpg import Connection
from services.database import create_conn, drop_tables_async
from waiting import wait


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
        p_rphp = Path(f"{Path(__file__).parent}/data/{phash_filename}").resolve()
        assert p_rphp.exists()
        return p_rphp

    return _resource_phash_path


@pytest.fixture(scope="session")
def db_is_ready(session_scoped_container_getter):
    # get database service
    bktreedb_service = next(
        service
        for service in session_scoped_container_getter.docker_project.services
        if service.name == "bktreedb"
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
