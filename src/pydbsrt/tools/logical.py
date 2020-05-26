#!/usr/bin/env python
# -*- coding: utf-8 -*-
# import itertools
# izip = getattr(itertools, 'izip', zip)
# izip_longest = getattr(itertools, 'izip_longest', itertools.zip_longest)
try:
    # Python 2
    from itertools import izip
    from itertools import izip_longest
except ImportError:
    # Python 3
    izip = zip
    from itertools import zip_longest as izip_longest

from bisect import bisect_left
from fuzzywuzzy import fuzz
from itertools import groupby
from operator import itemgetter
import operator


def mapping_merging(
        items,
        func_for_mapping,
        func_for_merging):
    """

    :param items:
    :param func_for_mapping:
    :param func_for_merging:
    :return:

    >>> from fuzzywuzzy import fuzz
    >>> import numpy as np
    >>> items = ['salut', 'chalut', 'au revoir', 'au re voir']
    >>> func_for_mapping = lambda t: fuzz.QRatio(*t) > 50
    >>> mapping_merging(items, func_for_mapping, np.logical_or)
    array([ True,  True,  True,  True], dtype=bool)
    """
    # Mapping
    mapping = list(map(lambda t: func_for_mapping(t), izip(items[:-1], items[1:]))) + [False]
    # Merging
    return func_for_merging(mapping, [False] + mapping[:-1])


def fuzzy_grouping(combined_list, threshold=80, fuzz_funcs=(fuzz.partial_ratio,)):
    """

    :param combined_list:
    :param threshold:
    :param fuzz_funcs:
    :type fuzz_funcs: tuple
    :return:

    https://stackoverflow.com/questions/35171710/how-to-group-words-whose-levenshtein-distance-is-more-than-80-percent-in-python
    """
    grs = list()
    for fuzz_func in fuzz_funcs:
        for name in combined_list:
            for g in grs:
                if all(fuzz_func(name, w) > threshold for w in g):
                    g.append(name)
                    break
            else:
                grs.append([name, ])

    return grs


def grouping_consecutive_numbers(it, func_grp_consumer=lambda g: list(map(itemgetter(1), g))):
    """

    Args:
        it (iter):
        func_grp_consumer (lambda):

    Returns:

    Examples:
        >>> grouping_consecutive_numbers([1, 2, 3, 4, 6, 7, 8, 11, 12])
        [[1, 2, 3, 4], [6, 7, 8], [11, 12]]
    """
    return [
        func_grp_consumer(g)
        for k, g in groupby(enumerate(it), lambda i_x: i_x[0] - i_x[1])
        ]


def gcn_with_slices(it):
    """

    Args:
        it:

    Returns:

    Examples:
        >>> gcn_with_slices([1, 2, 3, 4, 6, 7, 8, 11, 12])
        [(slice(0, 4, None), [1, 2, 3, 4]), (slice(4, 7, None), [6, 7, 8]), (slice(7, 9, None), [11, 12])]
    """
    return [
        (
            slice(g[0][0], g[-1][0]+1),
            list(map(itemgetter(1), g))
        )
        for g in grouping_consecutive_numbers(it, func_grp_consumer=lambda g: list(g))
    ]


def flatten_dict(dd, separator='_', prefix=''):
    """

    Args:
        dd (dict or Any):
        separator (str):
        prefix (str):

    Returns:
        result(dict): Flatten dict of 'dd'

    """
    return {
        prefix + separator + k if prefix else k: v
        for kk, vv in dd.items()
        for k, v in flatten_dict(vv, separator, kk).items()
    } if isinstance(dd, dict) else {prefix: dd}


# https://stackoverflow.com/questions/1624883/alternative-way-to-split-a-list-into-groups-of-n
def grouper_with_fill(n, iterable, fillvalue=None):
    """

    Args:
        n (int):
        iterable (iter):
        fillvalue (Any):

    Returns:
        grouper (list):

    Examples:
        >>> print(list(grouper_with_fill(3, 'ABCDEFG', 'x')))
        [('A', 'B', 'C'), ('D', 'E', 'F'), ('G', 'x', 'x')]
        >>> print(" ".join(map(lambda t: "".join(t), grouper_with_fill(3, 'ABCDEFG', 'x'))))
        ABC DEF Gxx
    """
    args = [iter(iterable)] * n
    return izip_longest(*args, fillvalue=fillvalue)


def grouper(iterable, n):
    """

    Args:
        iterable:
        n:

    Returns:

    """
    return grouper_with_fill(n, iterable)


# https://stackoverflow.com/questions/12141150/from-list-of-integers-get-number-closest-to-a-given-value
def takeclosest(ref_list, req_elmt):
    """
    Assumes myList is sorted. Returns closest value to myNumber.

    If two numbers are equally close, return the smallest number.
    """
    pos = bisect_left(ref_list, req_elmt)
    if pos == 0:
        return ref_list[0]
    if pos == len(ref_list):
        return ref_list[-1]
    before = ref_list[pos - 1]
    after = ref_list[pos]
    if after - req_elmt < req_elmt - before:
        return after
    else:
        return before


# https://stackoverflow.com/questions/2566412/find-nearest-value-in-numpy-array
def find_nearest1(array, value):
    """

    Args:
        array:
        value:

    Returns:

    """
    idx, val = min(enumerate(array), key=lambda x: abs(x[1] - value))
    return idx


def longest_common_subsequence(a, b, cmp=operator.eq, append=operator.add, init=""):
    """

    Args:
        a (iter):
        b (iter):
        cmp: Comparison function (equality)
        append:
        init (Any): Init value for result (depend of the type of datas containing in inputs sequences)

    Returns:
        result (iter): return longest common subsequence of a and b with a (potentially) customs comparison, append
        functions (with init value). The subsequence result is a subsequence of a sequence input (PS: not necessary
        equal with a subsequence in b ... it's a (potentially) consequence of fuzzy matching/comparison).
    """
    lengths = [[0 for _ in range(len(b)+1)] for _ in range(len(a)+1)]
    # row 0 and column 0 are initialized to 0 already
    for i, x in enumerate(a):
        for j, y in enumerate(b):
            if cmp(x, y):
                lengths[i+1][j+1] = lengths[i][j] + 1
            else:
                lengths[i+1][j+1] = max(lengths[i+1][j], lengths[i][j+1])
    # read the substring out from the matrix
    result = init
    x, y = len(a), len(b)
    while x != 0 and y != 0:
        if lengths[x][y] == lengths[x-1][y]:
            x -= 1
        elif lengths[x][y] == lengths[x][y-1]:
            y -= 1
        else:
            assert cmp(a[x-1], b[y-1])
            result = append(a[x-1], result)
            x -= 1
            y -= 1
    return result


def longest_common_subsequence_offsets(a, b, cmp=operator.eq, append=operator.add, init=""):
    """

    Args:
        a (iter):
        b (iter):
        cmp: Comparison function (equality)
        append:
        init (Any): Init value for result (depend of the type of datas containing in inputs sequences)

    Returns:
        result (iter): return longest common subsequence of a and b with a (potentially) customs comparison, append
        functions (with init value). The subsequence result is a subsequence of a sequence input (PS: not necessary
        equal with a subsequence in b ... it's a (potentially) consequence of fuzzy matching/comparison).

    Examples:
        # >>> longest_common_subsequence_offsets("abcdef", "acbef")

        >>> l1 = [14.80, 15.04, 20.04, 30.04, 16.92, 3.80, 24.04, 30.04, 10.04, 15.04, float('nan')]
        >>> l2 = [15, 20, 30, 21, 24, 30, 10, 15]
        >>> longest_common_subsequence_offsets(     \
                l1, l2,                             \
                cmp=lambda a, b: abs(a - b) <= 1,   \
                append=lambda a, b: [a] + b,        \
                init=[]                             \
        )
        [(0, 0), (2, 1), (3, 2), (6, 4), (7, 5), (8, 6), (9, 7)]
    """
    lengths = [[0 for _ in range(len(b)+1)] for _ in range(len(a)+1)]
    # row 0 and column 0 are initialized to 0 already
    for i, x in enumerate(a):
        for j, y in enumerate(b):
            if cmp(x, y):
                lengths[i+1][j+1] = lengths[i][j] + 1
            else:
                lengths[i+1][j+1] = max(lengths[i+1][j], lengths[i][j+1])
    # read the substring out from the matrix
    result = init
    x, y = len(a), len(b)
    while x != 0 and y != 0:
        if lengths[x][y] == lengths[x-1][y]:
            x -= 1
        elif lengths[x][y] == lengths[x][y-1]:
            y -= 1
        else:
            assert cmp(a[x-1], b[y-1])
            result = append((x-1, y-1), result)
            x -= 1
            y -= 1
    return result


def longest_common_subsequence_offsets_and_alternatives(a, b, cmp=operator.eq, append=operator.add, init=""):
    """

    Args:
        a (iter):
        b (iter):
        cmp: Comparison function (equality)
        append:
        init (Any): Init value for result (depend of the type of data containing in inputs sequences)

    Returns:
        result (iter): return longest common subsequence of a and b with a (potentially) customs comparison, append
        functions (with init value). The subsequence result is a subsequence of a sequence input (PS: not necessary
        equal with a subsequence in b ... it's a (potentially) consequence of fuzzy matching/comparison).

    Examples:
        >>> l1 = [14.80, 15.04, 20.04, 30.04, 16.92, 3.80, 24.04, 30.04, 10.04, 15.04, float('nan')]
        >>> l2 = [15, 20, 30, 21, 24, 30, 10, 15]
        >>> longest_common_subsequence_offsets_and_alternatives(     \
                l1, l2,                                         \
                cmp=lambda a, b: abs(a - b) <= 1,               \
                append=lambda a, b: [a] + b,                    \
                init=[]                                         \
        )
        ([(0, 0), (2, 1), (3, 2), (6, 4), (7, 5), (8, 6), (9, 7)], [[(1, 1)], [], [], [], [], [], []])
    """
    lengths = [[0 for _ in range(len(b)+1)] for _ in range(len(a)+1)]
    # row 0 and column 0 are initialized to 0 already
    for i, x in enumerate(a):
        for j, y in enumerate(b):
            if cmp(x, y):
                lengths[i+1][j+1] = lengths[i][j] + 1
            else:
                lengths[i+1][j+1] = max(lengths[i+1][j], lengths[i][j+1])
    # read the substring out from the matrix
    result = init
    x, y = len(a), len(b)
    alternatives = []
    subalternatives = []
    while x != 0 and y != 0:
        if lengths[x][y] == lengths[x-1][y]:
            if cmp(a[x - 1], b[y - 1]):
                subalternatives += [(x-1, y)]
            x -= 1
        elif lengths[x][y] == lengths[x][y-1]:
            if cmp(a[x - 1], b[y - 1]):
                subalternatives += [(x, y - 1)]
            y -= 1
        else:
            assert cmp(a[x-1], b[y-1])
            result = append((x-1, y-1), result)
            alternatives = append(subalternatives, alternatives)
            subalternatives = []
            x -= 1
            y -= 1
    return result, alternatives
