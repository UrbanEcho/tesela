from __future__ import annotations

from collections.abc import Mapping
from typing import Any

from typing_extensions import NotRequired, TypedDict


class QueryPayload(TypedDict):
    """Wire form of a search query. Pagination is keyset-only: no offset."""

    sort: list[dict[str, Any]]
    filter: NotRequired[dict[str, Any]]
    limit: NotRequired[int]
    cursor: NotRequired[str]
    fields: NotRequired[list[str]]


class TraversalPayload(QueryPayload):
    """Wire form of a traversal query: a search query plus the source key."""

    source_pk: object


class PageResult(TypedDict):
    """Records of one page; `next_cursor` is opaque and absent or None on the last page."""

    records: list[dict[str, Any]]
    next_cursor: NotRequired[str | None]


class SearchResult(TypedDict):
    """Result of `search` and `traverse`: one page plus the execution receipt."""

    output: PageResult
    receipt: dict[str, Any]


class TeselaError(Exception):
    code: str
    retryable: bool
    recovery: str | None
    side_effect: str
    resource_kind: str | None
    resource: str | None


class NativeMemoryBackend:
    def __init__(self) -> None: ...
    def bind_link(self, link: str, target_property: str) -> None: ...


class NativeRuntime:
    def __init__(self, spec: dict[str, Any], objects: Mapping[str, object], policy: object, *, links: Mapping[str, object] | None = ..., actions: Mapping[str, object] | None = ..., max_query_limit: int = ...) -> None: ...
    def spec(self) -> dict[str, Any]: ...
    def apply_spec(self, spec: dict[str, Any]) -> None: ...
    def search(self, context: dict[str, Any], object: str, query: QueryPayload) -> SearchResult: ...
    def get(self, context: dict[str, Any], object: str, key: object) -> dict[str, Any]: ...
    def lookup(self, context: dict[str, Any], object: str, key: object) -> dict[str, Any]: ...
    def mutate(self, context: dict[str, Any], object: str, mutation: dict[str, Any]) -> dict[str, Any]: ...
    def aggregate(self, context: dict[str, Any], object: str, query: dict[str, Any]) -> dict[str, Any]: ...
    def traverse(self, context: dict[str, Any], link: str, query: TraversalPayload) -> SearchResult: ...
    def execute_action(self, context: dict[str, Any], request: dict[str, Any]) -> dict[str, Any]: ...
    def capabilities(self, context: dict[str, Any], task: str) -> dict[str, Any]: ...
    def invoke(self, context: dict[str, Any], invocation: dict[str, Any]) -> dict[str, Any]: ...
    def explain(self, context: dict[str, Any], object: str, query: QueryPayload) -> dict[str, Any]: ...


def normalize_spec(spec: dict[str, Any]) -> dict[str, Any]: ...
