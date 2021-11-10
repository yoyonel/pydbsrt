import asyncio
import re
from pathlib import Path
from urllib.parse import urlparse

import imdb
from loguru import logger

from pydbsrt.tools.async_googlesearch import search


async def search_on_google(release_filename: str) -> str:
    logger.info(f"Google search on filename: {release_filename}")
    google_search_pattern = f"{release_filename} imdb"
    # https://pypi.org/project/googlesearch-python/
    google_search_results = await search(google_search_pattern, num_results=0)
    if not google_search_results:
        raise RuntimeError(f"No result for google search on {google_search_pattern=}'")
    imdb_url = google_search_results[0]
    return imdb_url


def extract_imdb_movie_id(imdb_url) -> str:
    # https://docs.python.org/3/library/urllib.parse.html#urllib.parse.urlparse
    imdb_path = urlparse(imdb_url).path
    logger.info(f"Extract IMDB movieID from URL path='{imdb_path}")
    # https://regex101.com/r/J6abAT/1
    regex = r"\/tt(?P<movieID>[0-9]+)"
    match = re.search(regex, imdb_path)
    if not match:
        raise RuntimeError(f"Can't extract movieID from {imdb_path=}")
    imdb_movie_id = match["movieID"]
    return imdb_movie_id


async def search_on_imdb(imdb_url: str) -> imdb.Movie.Movie:
    imdb_movie_id = extract_imdb_movie_id(imdb_url)
    logger.info(f"Get IMDB Movie from movieID={imdb_movie_id}")
    # TODO: need to move IMDB to asynchronous task (wrap this lib or create a real async lib)
    ia = imdb.IMDb()
    # https://imdbpy.readthedocs.io/en/latest/usage/quickstart.html#retrieving
    imdb_movie = ia.get_movie(imdb_movie_id)
    if not imdb_movie:
        raise RuntimeError(f"Can't get IMDB Movie from {imdb_movie_id=}")
    return imdb_movie


async def search_imdb_movie(release_path: Path) -> imdb.Movie.Movie:
    """

    :param release_path:
    :return:
    """
    if not release_path.exists():
        logger.error(f"{release_path=} doesn't exist !")
    release_filename = release_path.name
    imdb_url = await search_on_google(release_filename)
    return await search_on_imdb(imdb_url)


@logger.catch()
async def main():
    results = await asyncio.gather(
        *[
            search_imdb_movie(release_filename)
            for release_filename in (
                # "Seven.Samurai.1954.1080p.BluRay.x264-[YTS.AM].mp4",
                # "Space.Adventure.Cobra.1982.REPACK.1080p.BluRay.x264.AAC5.1-[YTS.MX].mp4",
                # "Gi-Juk.2021.1080p.WEBRip.x264.AAC-[YTS.MX].mp4",
                # "Primer.2004.1080p.WEBRip.x264-[YTS.AM].mp4",
                Path(
                    "/media/nas/volume1/video/Dallas Buyers Club (2013) [1080p]/Dallas.Buyers.Club.2013.1080p.BluRay.x264.YIFY.mp4"
                ),
                Path(
                    "/media/nas/volume1/video/Dallas Buyers Club (2013)[1080p-RARBG][en srt, pas fr]/Dallas.Buyers.Club.2013.1080p.BluRay.x265-RARBG.mp4"
                ),
            )
        ],
        return_exceptions=True,
    )
    for result in results:
        if isinstance(result, imdb.Movie.Movie):
            logger.info(result.summary())
        else:
            logger.error(result)


if __name__ == '__main__':
    loop = asyncio.get_event_loop()
    loop.run_until_complete(main())
    loop.close()
