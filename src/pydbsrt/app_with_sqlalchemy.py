"""
https://www.postgresql.org/docs/9.5/datatype.html
https://www.postgresql.org/docs/10/datatype-numeric.html

L'idée serait d'utiliser PostGreSQL + SPGIST comme moteur de recherche sur le phash
mais derrière on pourrait continuer d'utiliser une base nonsql (mongodb) pour gérer le reste -> toute la partie médias
"""
import logging
from dataclasses import dataclass
from itertools import islice
from pathlib import Path
from pprint import pformat
from random import choice
from typing import List

from imohash import hashfile
from sqlalchemy import create_engine, Column, BIGINT, INT, text, ForeignKey, TEXT
from sqlalchemy.ext.declarative import declarative_base
from sqlalchemy.orm import sessionmaker, relationship
from sqltap import sqltap
from tqdm import tqdm

from pydbsrt import settings
from pydbsrt.tools.ffmpeg_tools.ffmeg_extract_frame import ffmpeg_imghash_generator
from pydbsrt.tools.imghash import imghash_to_signed_int64
from pydbsrt.tools.timer_profiling import _Timer

LOG_FORMAT = "%(asctime)s - %(levelname)-8s - %(name)-13s - %(message)s"
logging.basicConfig(level=logging.INFO, format=LOG_FORMAT)
logger = logging.getLogger(__name__)

psqlDbIpAddr = settings.PSQL_IP
psqlDbName = settings.PSQL_DB_NAME
psqlUserName = settings.PSQL_USER
psqlUserPass = settings.PSQL_PASS
reset_table = False

db_string = f"postgres://{psqlUserName}:{psqlUserPass}@{psqlDbIpAddr}/{psqlDbName}"
db = create_engine(db_string)
base = declarative_base()


class TMedia(base):
    __tablename__ = "media"

    dbId = Column(INT, primary_key=True, autoincrement=True)
    mediaHash = Column(TEXT)
    mediaName = Column(TEXT)

    framesMedia = relationship("TFrameItem", backref='person', lazy='dynamic')


class TFrameItem(base):
    __tablename__ = "frameitems"

    dbId = Column(INT, primary_key=True, autoincrement=True)
    pHash = Column(BIGINT)

    frameOffset = Column(INT)
    mediaId = Column(INT, ForeignKey("media.dbId"))


@dataclass
class FrameItem:
    db_id: int
    p_hash: int
    frame_offset: int
    media_id: int


@dataclass
class FrameItemPaired:
    media_offset: int
    db_id: int


@dataclass
class MediaFrameItem:
    media_hash: int
    frame_offset: int


def insert_img_hashes_from_media_into_db(media_path: Path) -> List[FrameItemPaired]:
    assert media_path.exists()

    Session = sessionmaker(db)
    session = Session()

    # https://github.com/kalafut/py-imohash
    # (very) fast hash on file contents
    media_hash = hashfile(media_path, hexdigest=True)

    # Media already present in db ?
    # https://docs.sqlalchemy.org/en/14/orm/query.html#sqlalchemy.orm.Query.scalar
    if session.query(
            TMedia.mediaHash). \
            filter(TMedia.mediaHash == media_hash).scalar():
        logger.warning("Media='%s' with hash='%s' already present in DB", str(media_path), media_hash)
        return []

    media = TMedia(mediaHash=media_hash, mediaName=media_path.name)
    # TODO: chunkify
    logger.info("Build Image Hashes from media='%s'", str(media_path))
    frame_items = [TFrameItem(pHash=imghash_to_signed_int64(img_hash), frameOffset=offset_frame)
                   for offset_frame, img_hash in enumerate(tqdm(ffmpeg_imghash_generator(str(media_path))))]
    logger.info("Associate frame_items(len=%d) to media(=%s)", len(frame_items), media)
    media.framesMedia = frame_items
    logger.info("Add media to session and commit ...")
    session.add(media)
    session.commit()
    return [FrameItemPaired(i_frame_item, frame_item.dbId) for i_frame_item, frame_item in enumerate(frame_items)]


def search_img_hash_in_db(search_phash: int, search_distance: int = 0) -> List[FrameItem]:
    table_name = TFrameItem.__tablename__
    statement = text(
        f"""
SELECT "dbId", "pHash", "frameOffset", "mediaId" 
FROM "{table_name}" 
WHERE "pHash" <@ ({search_phash}, {search_distance})"""
    )
    with db.connect() as con:
        return [FrameItem(*tuple_frame_item) for tuple_frame_item in
                con.execute(statement)]


def create_indexes_in_db(db):
    with db.connect() as con:
        table_name = TFrameItem.__tablename__
        statement = f"""
CREATE INDEX IF NOT EXISTS {table_name}_phash_bk_tree_index ON {table_name} 
USING spgist ( "pHash" bktree_ops );"""
        result = con.execute(statement)
        logger.info(result)


def matching_with_db():
    with _Timer("Build ImageHashes list (from media)"):
        media_path = "data/big_buck_bunny_trailer_480p.webm"
        img_hashes = list(ffmpeg_imghash_generator(str(media_path)))
    img_hash_to_search = choice(img_hashes)
    logger.info("img_hash_to_search=%s", img_hash_to_search)
    params = dict(search_phash=imghash_to_signed_int64(img_hash_to_search),
                  search_distance=0)
    with _Timer("Search random ImageHash (from media) into DB"):
        matched_frame_items = search_img_hash_in_db(**params)
    logger.info("count(search_img_hash(%s)=%d", params,
                len(matched_frame_items))
    logger.info("matched_frame_items=%s", pformat(matched_frame_items))


def insert_medias_into_db():
    if reset_table:
        # https://askcodez.com/comment-supprimer-une-table-dans-sqlalchemy.html
        try:
            TFrameItem.__table__.drop(db)
            logger.info("%s table drop !", TFrameItem.__tablename__)
            TMedia.__table__.drop(db)
            logger.info("%s table drop !", TMedia.__tablename__)
        except:
            pass

    base.metadata.create_all(db)
    create_indexes_in_db(db)

    root_media_path = Path("/home/latty/NAS/tvshow/Silicon Valley/")
    for media_path in islice(root_media_path.glob("S02/*Homicide*.mkv"), 10):
        logger.info("media_path: %s", str(media_path))
        with _Timer("Insert ImageHashes (from media) into db"):
            _frames_items_paired = insert_img_hashes_from_media_into_db(media_path)


def do_db_processing():
    insert_medias_into_db()
    matching_with_db()


def profiling_do_db_processing():
    profiler = sqltap.start()
    do_db_processing()
    statistics = profiler.collect()
    try:
        sqltap.report(statistics, "report.html", report_format="html")
    except:
        logger.error("Can't export profiling report !", exc_info=True)


def main():
    # profiling_do_db_processing()
    do_db_processing()


if __name__ == "__main__":
    main()
