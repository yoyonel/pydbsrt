"""Media stem normalization and existing .phash / .ahash / certified coupling scanners."""

from __future__ import annotations

import json
import mmap
import sys
import unicodedata
import urllib.parse
from pathlib import Path
from typing import Set

from .models import (
    AMBIGUOUS_INTEGER_FPS_TOKENS,
    MAX_FPS_FRACTION_DIGITS,
    MAX_FPS_INTEGER_DIGITS,
    MAX_FPS_TOKEN_VALUE,
    MEDIA_NAME_RE,
    MIN_FPS_TOKEN_VALUE,
    MOVIEHASH_RE,
    VIDEO_EXTENSIONS,
)


def load_certified_stems(couplings_file: Path) -> Set[str]:
    """Loads all media stems already certified in couplings.json via read-only mmap regex scan."""
    stems: Set[str] = set()
    if couplings_file.is_file() and _file_size(couplings_file) > 0:
        try:
            with open(couplings_file, "rb") as fh, mmap.mmap(fh.fileno(), 0, access=mmap.ACCESS_READ) as data:
                for match in MEDIA_NAME_RE.finditer(data):
                    media_name = json.loads(b'"' + match.group(1) + b'"')
                    if media_name:
                        stems.add(media_name)
        except Exception as err:
            print(f"⚠️ Warning loading {couplings_file}: {err}", file=sys.stderr)
    return stems


def _is_valid_fps_int_part(token: str) -> bool:
    return (
        1 <= len(token) <= MAX_FPS_INTEGER_DIGITS and token.isdigit() and not (len(token) > 1 and token.startswith("0"))
    )


def strip_fps_token(without_ext: str) -> str:
    """Strips a trailing `.<fps>fps` suffix without amputating release years or episode numbers."""
    if not without_ext.endswith("fps"):
        return without_ext
    before_fps = without_ext[:-3]
    last_dot = before_fps.rfind(".")
    if last_dot < 0:
        return without_ext
    tail = before_fps[last_dot + 1 :]
    prev_dot = before_fps.rfind(".", 0, last_dot)
    if prev_dot >= 0:
        int_part = before_fps[prev_dot + 1 : last_dot]
        if (
            _is_valid_fps_int_part(int_part)
            and 1 <= len(tail) <= MAX_FPS_FRACTION_DIGITS
            and tail.isdigit()
            and tail not in AMBIGUOUS_INTEGER_FPS_TOKENS
        ):
            val = float(f"{int_part}.{tail}")
            if MIN_FPS_TOKEN_VALUE <= val <= MAX_FPS_TOKEN_VALUE:
                return before_fps[:prev_dot]
    if _is_valid_fps_int_part(tail):
        val = float(tail)
        if MIN_FPS_TOKEN_VALUE <= val <= MAX_FPS_TOKEN_VALUE:
            return before_fps[:last_dot]
    return without_ext


def ahash_logical_stem(ahash_name: str) -> str:
    """Logical stem matching Rust `ahash_logical_stem`: strips `.ahash` and trailing `.<fps>fps`."""
    return strip_fps_token(ahash_name.removesuffix(".ahash"))


def phash_media_stem(phash_name: str) -> str:
    """Media stem of a .phash or .ahash file name without URL encoding, fps token, or video ext."""
    unquoted = urllib.parse.unquote(phash_name).removesuffix(".phash").removesuffix(".ahash")
    raw_name = strip_fps_token(unquoted)
    for ext in VIDEO_EXTENSIONS:
        if raw_name.lower().endswith(ext):
            return raw_name[: -len(ext)]
    return raw_name


def _file_size(path: Path) -> int:
    try:
        return path.stat().st_size
    except OSError:
        return 0


def _valid_ahash_sidecar_media_name(ahash: Path) -> str | None:
    """Reads `<stem>.json` beside `<stem>.<fps>fps.ahash` and validates typed AhashSidecar fields."""
    sidecar = ahash.with_name(f"{ahash_logical_stem(ahash.name)}.json")
    if _file_size(sidecar) <= 0:
        return None
    try:
        payload = json.loads(sidecar.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    if not isinstance(payload, dict):
        return None
    media_name = payload.get("media_name")
    selection = payload.get("selection")
    header = payload.get("header")
    if (
        not isinstance(media_name, str)
        or not media_name.strip()
        or not isinstance(selection, str)
        or not isinstance(header, dict)
    ):
        return None
    return media_name


def _sidecar_media_name(phash: Path, fallback_dir: Path | str | None = None) -> str:
    """Returns `media_name` from `<moviehash>.json` sidecar next to `<moviehash>.<fps>fps.phash`."""
    head = phash.name.split(".", 1)[0]
    if not MOVIEHASH_RE.fullmatch(head):
        return ""
    candidates = [phash.with_name(f"{head}.json")]
    if fallback_dir:
        candidates.append(Path(fallback_dir) / f"{head}.json")
    for sidecar in candidates:
        try:
            if sidecar.is_file():
                media_name = json.loads(sidecar.read_text(encoding="utf-8")).get("media_name")
                if isinstance(media_name, str):
                    return media_name
        except (OSError, ValueError, AttributeError):
            continue
    return ""


def load_existing_phash_stems(phash_dir: Path) -> Set[str]:
    """Loads all media stems with existing non-empty .phash files on disk."""
    stems: Set[str] = set()
    if phash_dir.is_dir():
        for path in phash_dir.glob("*.phash"):
            if _file_size(path) > 0:
                stems.update((phash_media_stem(path.name), path.stem))
                media_name = _sidecar_media_name(path)
                if media_name:
                    stems.add(phash_media_stem(media_name))
    return stems


def load_existing_ahash_stems(ahash_dir: Path) -> Set[str]:
    """Loads media stems having BOTH a non-empty .ahash file and a valid typed `<stem>.json` sidecar."""
    stems: Set[str] = set()
    if ahash_dir.is_dir():
        for path in ahash_dir.glob("*.ahash"):
            if _file_size(path) <= 0:
                continue
            media_name = _valid_ahash_sidecar_media_name(path)
            if not media_name:
                continue
            stems.update((phash_media_stem(path.name), path.stem, phash_media_stem(media_name)))
    return stems


def resolve_done_stems(
    mode: str,
    certified_stems: Set[str],
    existing_phash: Set[str],
    existing_ahash: Set[str],
) -> Set[str]:
    """Filters completed stems according to the target mode (ahash, phash, or both)."""
    if mode == "ahash":
        print(f"✓ Found {len(existing_ahash)} .ahash files locally (mode: ahash).")
        return {unicodedata.normalize("NFC", stem) for stem in existing_ahash}
    if mode == "phash":
        print(
            f"✓ Found {len(certified_stems)} certified stems and {len(existing_phash)} .phash files locally (mode: phash)."
        )
        return {unicodedata.normalize("NFC", stem) for stem in certified_stems | existing_phash}
    print(f"✓ Found {len(existing_phash)} .phash and {len(existing_ahash)} .ahash files locally (mode: both).")
    return {unicodedata.normalize("NFC", stem) for stem in (certified_stems | existing_phash) & existing_ahash}
