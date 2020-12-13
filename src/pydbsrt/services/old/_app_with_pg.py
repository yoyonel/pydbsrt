"""
https://www.postgresql.org/docs/9.5/datatype.html
https://www.postgresql.org/docs/10/datatype-numeric.html

L'idée serait d'utiliser PostGreSQL + SPGIST comme moteur de recherche sur le phash
mais derrière on pourrait continuer d'utiliser une base nonsql (mongodb) pour gérer le reste -> toute la partie médias
"""
import logging
from contextlib import contextmanager
from pathlib import Path

import psycopg2
import sql
from tqdm import tqdm

from pydbsrt import settings
from pydbsrt.tools.ffmpeg_tools.ffmeg_extract_frame import ffmpeg_imghash_generator
from pydbsrt.tools.imghash import imghash_to_signed_int64

psqlDbIpAddr = settings.PSQL_IP
psqlDbName = settings.PSQL_DB_NAME
psqlUserName = settings.PSQL_USER
psqlUserPass = settings.PSQL_PASS

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)


def connect():
    try:
        conn = psycopg2.connect(
            dbname=psqlDbName, user=psqlUserName, password=psqlUserPass
        )

    except psycopg2.OperationalError:
        conn = psycopg2.connect(
            host=psqlDbIpAddr,
            dbname=psqlDbName,
            user=psqlUserName,
            password=psqlUserPass,
        )
    return conn


@contextmanager
def transaction(conn, commit=True):
    cursor = conn.cursor()

    if commit:
        cursor.execute("BEGIN;")

    try:
        yield cursor

    except Exception as e:
        if commit:
            cursor.execute("ROLLBACK;")
        raise e

    finally:
        if commit:
            cursor.execute("COMMIT;")


def init_db(conn):
    table_name = "frameitems"

    with transaction(conn) as cursor:
        cursor.execute(
            "SELECT * FROM information_schema.tables WHERE table_name=%s", (table_name,)
        )
        have = cursor.fetchall()
        main_table_exists = bool(have)
    if not main_table_exists:
        logger.info("Need to create table!")
        cursor.execute(
            f"""
CREATE TABLE IF NOT EXISTS {table_name} (
    dbId            SERIAL PRIMARY KEY,
    pHash           BIGINT
);"""
        )
        logger.info("Creating indexes")
        cursor.execute(
            f"CREATE        INDEX {table_name}_phash_index  ON {table_name} USING spgist (pHash bktree_ops)"
        )
        logger.info("Done!")
    return sql.Table(table_name.lower())


def insert_img_hashes_into_db(input_media_path: Path, conn, table: sql.Table):
    """"""
    signed_int64_img_hashes = [
        imghash_to_signed_int64(img_hash)
        for img_hash in tqdm(ffmpeg_imghash_generator(str(input_media_path)))
    ]
    with transaction(conn) as cursor:
        insert = """
    insert into frameitems (pHash)
    select pHash
    from unnest(%s) s(pHash bigint)
    returning id;"""
        cursor.execute(insert, (signed_int64_img_hashes,))
        logger.info(cursor.fetchall())
        conn.commit()


def main():
    conn = connect()
    table = init_db(conn)
    media_path = Path("data/big_buck_bunny_trailer_480p.webm")
    assert media_path.exists()
    insert_img_hashes_into_db(media_path, conn, table)


if __name__ == "__main__":
    main()
