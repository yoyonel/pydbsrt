"""Logical name of a .phash file, legacy-named or named by moviehash with its sidecar."""

from __future__ import annotations

import json
from pathlib import Path


def phash_logical_name(phash: Path) -> str:
    """<media_name>.<fps>fps.phash for a <moviehash>.<fps>fps.phash with its <moviehash>.json sidecar, else the file name.

    Same rule as dbsrt_core::phash_logical_name.
    """
    head, _, rest = phash.name.partition(".")
    if len(head) != 16 or any(c not in "0123456789abcdefABCDEF" for c in head):
        return phash.name
    try:
        media_name = json.loads(phash.with_name(f"{head}.json").read_text(encoding="utf-8")).get("media_name")
    except (OSError, ValueError, AttributeError):
        return phash.name
    return f"{media_name}.{rest}" if isinstance(media_name, str) and media_name else phash.name


def phash_media_name(phash: Path) -> str:
    """phash_logical_name without its .phash extension: what a legacy name's Path.stem gave."""
    return phash_logical_name(phash).removesuffix(".phash")
