"""Runtime facade registry preserving monkeypatch compatibility on o2switch_batch_worker."""

from __future__ import annotations

from types import ModuleType
from typing import TypeVar

T = TypeVar("T")

_FACADE_MODULE: ModuleType | None = None


def register_facade(module: ModuleType) -> None:
    """Registers the top-level o2switch_batch_worker module for live attribute resolution."""
    global _FACADE_MODULE
    _FACADE_MODULE = module


def facade_attr(name: str, default: T) -> T:
    """Returns attribute `name` from registered facade module if present, else `default`."""
    if _FACADE_MODULE is not None and hasattr(_FACADE_MODULE, name):
        return getattr(_FACADE_MODULE, name)  # type: ignore[no-any-return]
    return default
