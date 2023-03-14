from typing import Optional, Union

from asyncpg import Connection


async def search_media_hash_in_db(conn: Connection, media_hash: Union[str, bytes]) -> Optional[int]:
    """

    Args:
        conn:
        media_hash:

    Returns:

    """
    # https://magicstack.github.io/asyncpg/current/api/index.html?highlight=returning#asyncpg.connection.Connection.fetchval
    found_media_id = await conn.fetchval(
        """
            SELECT
                id
            FROM
                medias
            WHERE
                medias.media_hash = $1;
        """,
        media_hash,
    )
    return found_media_id
