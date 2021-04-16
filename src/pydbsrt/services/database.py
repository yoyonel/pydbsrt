from rich.console import Console

from pydbsrt import settings

psqlDbIpAddr = settings.PSQL_IP
psqlDbName = settings.PSQL_DB_NAME
psqlUserName = settings.PSQL_USER
psqlUserPass = settings.PSQL_PASS

console = Console()


async def drop_tables(conn):
    for table_name in ("frames", "medias"):
        await conn.execute(f"""DROP TABLE IF EXISTS {table_name} CASCADE;""")


async def create_tables(conn):
    """
    one-to-many: media ->* frames
    """
    await conn.execute(
        """
                CREATE TABLE IF NOT EXISTS medias (
                    id              SERIAL PRIMARY KEY,
                    media_hash      CHAR(32) UNIQUE,
                    -- https://www.postgresqltutorial.com/postgresql-char-varchar-text/
                    name            TEXT
                )"""
    )
    await conn.execute(
        """
                CREATE TABLE IF NOT EXISTS frames (
                    id              SERIAL PRIMARY KEY,
                    p_hash          BIGINT,
                    frame_offset    INT,
                    media_id        INT,
                    FOREIGN KEY (media_id) REFERENCES medias(id) ON DELETE CASCADE
                )"""
    )


async def create_indexes(conn):
    await conn.execute(
        """
                CREATE INDEX IF NOT EXISTS frames_phash_bk_tree_index ON frames
                USING spgist ( p_hash bktree_ops );"""
    )


async def search_img_hash(conn, search_phash=-6023947298048657955, search_distance=1):
    values = await conn.fetch(
        f"""
                SELECT "id", "p_hash", "frame_offset", "media_id"
                FROM "frames"
                WHERE "p_hash" <@ ({search_phash}, {search_distance})"""
    )
    console.print(
        f"count(searching(phash={search_phash}, search_distance={search_distance}))={len(values)}"
    )
