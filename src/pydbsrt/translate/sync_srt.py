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
from typing import Generator, Tuple
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


def translate_srt_with_google_translate_webapi(
        srt_en_path,
        it_func=lambda it, *args, **kwargs: it
):
    srt_en = pysrt.open(srt_en_path)

    # big chunk
    # TODO: need to find the maximum size allowed to transfer
    # Pb: hard to control the separation (and reconstruction) of blob text for
    # translation ... google translate seems to do strange things
    # (unification, interpretation) with the split_token ...
    # This behaviours disturbed the translation and introduce offsets on (block of)
    # translations reduce chuck to unit => very slow but working :-/
    chunk_size = 10
    # TODO: Trying to find a better split-token
    split_token = ' ¶ '

    gen_slice_srt_text = map(
        lambda g: split_token.join(map(lambda s: ''.join(clean_html(s)), g)),
        grouper_without_fill(map(lambda srt: srt.text, srt_en), chunk_size)
    )
    for slice_srt_text in it_func(gen_slice_srt_text, total=len(srt_en) // chunk_size):
        try:
            t_slice_srt_text = unescape(
                translate_with_google_translate_webapi(slice_srt_text, from_l='en', to_l='fr'))
        except Exception as e:
            break
        else:
            # https://stackoverflow.com/questions/44780357/how-to-use-newline-n-in-f-string-to-format-output-in-python-3-6
            #         t_srt_text = "\n¶ ".join(t_slice_srt_text.split('¶'))
            #         print(f"{t_srt_text}")
            pp.pprint(list(zip(slice_srt_text.split(split_token),
                               t_slice_srt_text.split(split_token))))


def main():
    # test_with_google_translate()
    # test_synchronize_subtitles()
    # translate_subtitles_with_google(
    #     medias_root_path / "The Golden Child (1986) 1080p web.en.srt"
    # )

    # logger.info(translate("Salut le monde", to_l='en', from_l='fr'))
    translate_srt_with_google_translate_webapi(
        medias_root_path / "The Golden Child (1986) 1080p web.en.srt",
        it_func=tqdm
    )


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
