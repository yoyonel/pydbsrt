"""Typed dataclasses and query criteria models for pydbsrt Portal."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from pydbsrt.portal.config import DEFAULT_PAGE_SIZE, DEFAULT_TRAJECTORY_PAGE_SIZE


@dataclass
class PhashReconciliationContext:
    """Context holding sidecars and PostgreSQL lookups for fast tripartite alignment."""

    sidecars: dict[str, dict[str, Any]] = field(default_factory=dict)
    sidecar_name_index: dict[str, str] = field(default_factory=dict)
    pg_medias: dict[str, dict[str, Any]] = field(default_factory=dict)


@dataclass
class PhashesQueryCriteria:
    """Query criteria for filtering, sorting and paginating pHash tripartite items."""

    page: int = 1
    limit: int = DEFAULT_PAGE_SIZE
    status_filter: str = "all"
    search_q: str = ""
    sort_by: str = "id_asc"


@dataclass
class TrajectoriesQueryCriteria:
    """Query criteria for filtering, sorting and paginating spatiotemporal trajectory records."""

    file_name: str = ""
    page: int = 1
    limit: int = DEFAULT_TRAJECTORY_PAGE_SIZE
    status_filter: str = "all"
    search_q: str = ""
    sort_by: str = "name_asc"


@dataclass
class AssetsQueryCriteria:
    """Query criteria for filtering, sorting and paginating PostgreSQL media assets."""

    page: int = 1
    limit: int = DEFAULT_PAGE_SIZE
    status_filter: str = "all"
    source_filter: str = ""
    search_q: str = ""
    sort_by: str = "title_asc"
