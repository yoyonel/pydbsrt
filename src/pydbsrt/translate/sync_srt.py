#!/usr/bin/env python
"""
"""
import logging
import re
import sys
import urllib.request
from functools import partial
from html import unescape  # https://docs.python.org/3/library/html.html
from itertools import zip_longest
from operator import is_not
from pathlib import Path
from pprint import PrettyPrinter, pformat
from typing import Generator, List, Tuple
from urllib.parse import quote

import pysrt
from fuzzywuzzy import fuzz
from googletrans import Translator
from pysrt import SubRipItem
from tqdm import tqdm

from pydbsrt.tools.tqdm_with_logger import TqdmLoggingHandler

logger = logging.getLogger(__name__)

medias_root_path = Path('data/')

typ = sys.getfilesystemencoding()
clean_r = re.compile('<.*?>')

pp = PrettyPrinter(indent=4)


def grouper(iterable, n, fillvalue=None):
    """Collect data into fixed-length chunks or blocks

    >>> " ".join(["".join(g) for g in grouper('ABCDEFG', 3, 'x')])
    'ABC DEF Gxx'
    """
    args = [iter(iterable)] * n
    return zip_longest(*args, fillvalue=fillvalue)


def grouper_without_fill(iterable, n):
    """Collect data into fixed-length chunks or blocks

    >>> " ".join(["".join(g) for g in grouper_without_fill('ABCDEFG', 3)])
    'ABC DEF G'
    """
    # https://stackoverflow.com/questions/16096754/remove-none-value-from-a-list-without-removing-the-0-value
    # https://stackoverflow.com/questions/40137950/possible-to-run-python-doctest-on-a-jupyter-cell-function
    # return map(lambda g: list(filter(lambda e: e is not None, g)),
    return map(lambda g: filter(partial(is_not, None), g),
               grouper(iterable, n, fillvalue=None))


def clean_html(raw_html):
    clean_text = re.sub(clean_r, '', raw_html)
    return clean_text


def gen_translate_srt(
        subs_src,
        src='en',
        dest='fr',
        it_func=lambda it: it,
) -> Generator[Tuple[SubRipItem, str], None, None]:
    translator = Translator()
    for sub in it_func(subs_src):
        yield (sub, translator.translate(text=sub.text, src=src, dest=dest).text)


def test_with_google_translate():
    fn_subs_en = "Teenage.Mutant.Ninja.Turtles.1990.1080p.BluRay.H264.AAC-RARBG.srt"
    fn_subs_fr = "Teenage.Mutant.Ninja.Turtles.1990.iNTERNAL.DV-VH-PROD.french.srt"

    subs_en = pysrt.open(medias_root_path / fn_subs_en)
    subs_fr = pysrt.open(medias_root_path / fn_subs_fr)

    translator = Translator()
    subs_en_fr = [
        (sub, translator.translate(text=sub.text, src='en', dest='fr').text)
        for sub in tqdm(subs_en[:10])
    ]

    # gen_subs_en_fr = gen_translate_srt(subs_en[:10], it_func=tqdm)
    # subs_en_fr = list(gen_subs_en_fr)

    logger.info(pformat(([(sub[0].text, sub[1]) for sub in subs_en_fr])))

    # be sure to validate the synchronization
    if fuzz.partial_ratio(subs_en_fr[0][1], subs_fr[0].text) <= 60:
        logger.error(f"SRTs (en: {fn_subs_en}, fr: {fn_subs_fr}) "
                     f"seems to be not synchronize")

    # Process synchronization
    if subs_en[0].start >= subs_fr[0].start:
        shift = (subs_en[0].start - subs_fr[0].start)
    else:
        shift = (subs_fr[0].start - subs_en[0].start)
    logger.info(f"shift: {shift}")
    # subs_fr_shifted = subs_fr.copy()
    # subs_fr_shifted.shift(minutes=shift.minutes, seconds=shift.seconds,
    #                       milliseconds=shift.milliseconds)


# # https://stackoverflow.com/questions/3992735/python-generator-that-groups-another-iterable-into-groups-of-n/3992765
# # https://stackoverflow.com/questions/31164731/python-chunking-csv-file-multiproccessing/31170795#31170795
# # https://docs.python.org/3/library/functions.html#iter
# def grouper(n, iterable):
#     """
#     >>> list(grouper(3, 'ABCDEFG'))
#     [['A', 'B', 'C'], ['D', 'E', 'F'], ['G']]
#     """
#     iterable = iter(iterable)
#     return iter(lambda: list(IT.islice(iterable, n)), [])


def translate_subtitles_with_google(
        srt_path: Path,
        srt_src: str = 'en',
        srt_dest: str = 'fr',
):
    """

    :param srt_path:
    :param srt_src:
    :param srt_dest:
    :return:
    """
    srt = pysrt.open(srt_path)
    # compute subtitles translation, can be long ...
    it_srt_translated = grouper(50, gen_translate_srt(srt, src=srt_src, dest=srt_dest))
    # translate_srt = list(
    #     grouper(50, gen_translate_srt(srt, src=srt_src, dest=srt_dest))
    # )
    first_50_translate_srt = next(it_srt_translated)
    logger.info(f"first_50_translate_srt: \n{first_50_translate_srt}")


def test_synchronize_subtitles():
    srt_fr_fn = "The Golden Child.DVD_PAL.fre.srt"
    srt_en_fn = "The Golden Child (1986) 1080p web.en.srt"

    srt_fr = pysrt.open(medias_root_path / srt_fr_fn)
    srt_en = pysrt.open(medias_root_path / srt_en_fn)

    ####################################################################################
    # see problems
    # trying changing frequency ratio
    srt_fr.shift(ratio=25.00 / 23.9)
    diff_starts = srt_fr[2].start - srt_en[0].start
    # seems to work ...
    logger.info(f"srt_fr[2].start - srt_en[0].start: {diff_starts}")

    # but ... we see the shifting over the time
    diff_starts = [
        (srt_fr[i + 2].text,
         srt_en[i].text,
         srt_fr[i + 2].start - srt_en[i].start)
        for i in range(100)
    ]
    logger.info(pformat(diff_starts))
    ####################################################################################

    srt_en_fr = []
    for i, srt_translated in enumerate(gen_translate_srt(srt_en, it_func=tqdm)):
        srt_en_fr.append(srt_translated)
        #
        srt_from, srt_to = srt_translated
        pp.pprint(
            (
                i,
                srt_from.text,
                srt_to
            )
        )


def translate_with_google_translate_webapi(querystr, to_l="zh", from_l="en"):
    """
    for google tranlate by doom

    Jupyter Notebook - Google Translate (by web API):
    https://gist.github.com/yoyonel/78081a1466121617ff27c307a79b82cd#file-google_translate-ipynb

    :param querystr:
    :param to_l:
    :param from_l:
    :return:
    """
    c_agent = {
        'User-Agent': "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, "
                      "like Gecko) Chrome/31.0.165063 Safari/537.36 AppEngine-Google."}
    flag = 'class="t0">'
    translate_google_url = "http://translate.google.cn"
    tar_url = f"m?hl={to_l}&sl={from_l}&q={quote(querystr, safe='')}"
    request = urllib.request.Request(f"{translate_google_url}/{tar_url}",
                                     headers=c_agent)
    page = str(urllib.request.urlopen(request).read().decode(typ))
    target = page[page.find(flag) + len(flag):]
    target = target.split("<")[0]
    return target


def translate_subtitles_with_google_translate_webapi(
        srt_from_path,
        str_from_lang: str = 'en',
        str_to_lang: str = 'fr',
        chunk_size: int = 10,
        split_tokens: List[str] = [' ¶ '],
        it_func=lambda it, *args, **kwargs: it,
) -> Generator[Tuple[SubRipItem, str], None, None]:
    """

    Pb: hard to control the separation (and reconstruction) of blob text for
    translation ... google translate seems to do strange things
    (unification, interpretation) with the split_token ...
    This behaviours disturbed the translation and introduce offsets on (block of)
    translations reduce chuck to unit => very slow but working :-/

    Bugs:
        - chunk_size=4
        => Le log/srt 40 semble disparaitre ...
            2019-05-30 18:12:21,866 - __main__ - INFO - 39
            00:10:35,498 --> 00:10:39,764
            <i>Now you can talk with the tortoises.</i>
            <i>Excuse me.</i>
                -> Je vous remercie. C'était Chandler Jarrell.

            2019-05-30 18:12:22,033 - __main__ - INFO - 41

        - chunk_size=5
            00:10:35,498 --> 00:10:39,764
            <i>Now you can talk with the tortoises.</i>
            <i>Excuse me.</i>
                -> Je vous remercie. C'était Chandler Jarrell.

    Args:
        srt_from_path:
        str_from_lang:
        str_to_lang:
        chunk_size: (TODO: need to find the maximum size allowed to transfer)
        split_token: (TODO: Trying to find a better split-token)
        it_func:

    Returns:

    """
    srt_from = pysrt.open(srt_from_path)
    gen_grp_srt_from = grouper_without_fill(srt_from, chunk_size)
    gen_grp_srt_from = list(gen_grp_srt_from)
    nb_srt_from = len(srt_from)
    id_chunk = 0
    for grp_srt_from in it_func(gen_grp_srt_from, total=len(srt_from) // chunk_size):
        nb_chunk_to_find = min(chunk_size, nb_srt_from - id_chunk * chunk_size)

        # https://docs.python.org/2/library/itertools.html#itertools.tee
        # (grp_srt_from_for_translation,
        #  grp_srt_from_for_zip) = itertools.tee(grp_srt_from)
        grp_srt_from = list(grp_srt_from)
        grp_srt_from_for_translation, grp_srt_from_for_zip = grp_srt_from, grp_srt_from
        t_grp_srt_from = []
        split_token = split_tokens[0]
        for split_token in split_tokens:
            t_grp_srt_from = unescape(
                translate_with_google_translate_webapi(
                    split_token.join(
                        map(lambda s: clean_html(
                            s.text_without_tags.replace('\n', ' ')),
                            grp_srt_from_for_translation)
                    ),
                    from_l=str_from_lang, to_l=str_to_lang))
            nb_chunks_produced = len(t_grp_srt_from.split(split_token))
            if nb_chunks_produced == nb_chunk_to_find:
                break
            logger.warning(f'split token=\'{split_token}\' '
                           f'produce {nb_chunks_produced} chunks (!= {chunk_size})')
        if len(t_grp_srt_from.split(split_token)) == nb_chunk_to_find:
            for t_srt_from, srt_from in zip(t_grp_srt_from.split(split_token),
                                            grp_srt_from_for_zip):
                yield srt_from, t_srt_from
        else:
            logger.error("Can't find valid token to split the subtitles")
        id_chunk += 1


def main():
    # test_with_google_translate()
    # test_synchronize_subtitles()
    # translate_subtitles_with_google(
    #     medias_root_path / "The Golden Child (1986) 1080p web.en.srt"
    # )

    # logger.info(translate("Salut le monde", to_l='en', from_l='fr'))

    srt_to_path = medias_root_path / "The Golden Child.DVD_PAL.fre.srt"
    srt_to = pysrt.open(srt_to_path)

    for srt_from, t_srt_from in translate_subtitles_with_google_translate_webapi(
            medias_root_path / "The Golden Child (1986) 1080p web.en.srt",
            chunk_size=70,
            split_tokens=[' ¶ ', ' ** ', ' @ ', ' @_ '],
            # it_func=tqdm
    ):
        logger.info(f"\n{str(srt_from)}-> {t_srt_from}\n")

        # srt_scores_matching = [fuzz.partial_ratio(s_to.text, t_srt_from)
        #                        for s_to in srt_to]

        # try:
        #     best_match = max(
        #         [
        #             (s[0], [w for w in s[1] if len(w) >= 4])
        #             for s in filter(
        #             lambda s: s[1] != set(),
        #             [
        #                 (i, set(t_srt_from.split()).intersection(s_to.text.split()))
        #                 for i, s_to in enumerate(srt_to)
        #             ],
        #         )
        #         ],
        #         key=lambda s: len("".join(s[1]))
        #     )
        #     logger.info(f"best match found: {best_match}\n->{srt_to[best_match[0]]}")
        # except ValueError:
        #     logger.warning(f"No match found for: {t_srt_from}")

        # best_matchs = [
        #     f"{srt_to[match[0]]}"
        #     for match in filter(
        #         lambda i_s: i_s[1] > 20,
        #         enumerate([fuzz.ratio(t_srt_from, s.text_without_tags)
        #                    for s in srt_to])
        #     )
        # ]
        best_matchs = max(
            filter(
                lambda i_s: i_s[1] > 20,
                enumerate(
                    [
                        fuzz.ratio(t_srt_from, s.text_without_tags)
                        for s in srt_to
                    ]
                )
            ),
            key=lambda m: m[1]
        )
        logger.info(
            f"best matchs for '{t_srt_from}':"
            f"\n{srt_to[best_matchs[0]]} - score = {best_matchs[1]}")


def init_logger():
    logger.setLevel(logging.INFO)
    tqdm_logging_handler = TqdmLoggingHandler()
    formatter = logging.Formatter(
        '%(asctime)s - %(name)s - %(levelname)s - %(message)s')
    tqdm_logging_handler.setFormatter(formatter)
    logger.addHandler(tqdm_logging_handler)


if __name__ == '__main__':
    init_logger()
    main()
