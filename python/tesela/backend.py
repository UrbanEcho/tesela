"""Typed repository and native backend contracts."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Callable, Generic, Iterable, Mapping, TypeVar

from .context import Context

T = TypeVar("T")


@dataclass(frozen=True, slots=True)
class Changes(Generic[T]):
    values: Mapping[str, Any]


@dataclass(frozen=True, slots=True)
class Repository(Generic[T]):
    scan: Callable[[Context], Iterable[T]] | None = None
    get: Callable[[Context, Any], T | None] | None = None
    create: Callable[[Context, T], T] | None = None
    update: Callable[[Context, Any, Changes[T]], T] | None = None
    delete: Callable[[Context, Any], None] | None = None


class Native(Generic[T]):
    """Base class for integrations that execute planned queries natively.

    `search` must honour the query's filter, sort and limit and return the rows
    ordered by the sort keys followed by the primary key ascending (an unsorted
    query is ordered by primary key). Missing and null values form one class that
    sorts before present values ascending and after them descending. Cursor
    resumption arrives already folded into `query["filter"]`: return the first
    `limit` matching rows in that order and nothing else. The runtime asks for one
    extra row to detect the next page, verifies the ordering and derives the
    opaque cursor itself, so `search` returns only records and never a cursor.
    """

    def search(self, context: Context, query: Mapping[str, Any]) -> Iterable[T]:
        raise NotImplementedError

    def aggregate(self, context: Context, query: Mapping[str, Any]) -> Mapping[str, Any]:
        raise NotImplementedError


class BackendNamespace:
    """Callable repository factory plus backend integration types."""

    Changes = Changes
    Native = Native
    Repository = Repository

    def __call__(self, *, scan: Callable[[Context], Iterable[T]] | None = None,
                 get: Callable[[Context, Any], T | None] | None = None,
                 create: Callable[[Context, T], T] | None = None,
                 update: Callable[[Context, Any, Changes[T]], T] | None = None,
                 delete: Callable[[Context, Any], None] | None = None) -> Repository[T]:
        if not any((scan, get, create, update, delete)):
            raise ValueError("backend requires at least one callback")
        return Repository(scan, get, create, update, delete)


backend: BackendNamespace = BackendNamespace()

__all__ = ["BackendNamespace", "Changes", "Native", "Repository", "backend"]
