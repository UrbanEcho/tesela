"""Immutable typed plans. Plans never execute themselves."""

from __future__ import annotations

from dataclasses import dataclass, replace
from types import MappingProxyType
from typing import Any, Generic, Iterator, Mapping, TypeVar

from .expr import AggregateExpr, Expr, Order
from .model import LinkDefinition

T = TypeVar("T")
S = TypeVar("S")


@dataclass(frozen=True, slots=True)
class Page(Generic[T]):
    items: tuple[T, ...]
    next_cursor: str | None = None

    def __iter__(self) -> Iterator[T]:
        return iter(self.items)


@dataclass(frozen=True, slots=True)
class Execution(Generic[T]):
    value: T
    receipt: Mapping[str, Any]
    revision: str | None = None


@dataclass(frozen=True, slots=True)
class AggregationResult:
    groups: tuple[Mapping[str, Any], ...]


@dataclass(frozen=True, slots=True)
class BatchResult:
    rows_affected: int | None


@dataclass(frozen=True, slots=True)
class Query(Generic[T]):
    model: type[T]
    predicates: tuple[Expr[bool], ...] = ()
    orders: tuple[Order, ...] = ()
    page_limit: int | None = None
    page_offset: int | None = None

    def where(self, *predicates: Expr[bool]) -> Query[T]:
        if not predicates:
            raise ValueError("where requires at least one predicate")
        return replace(self, predicates=self.predicates + predicates)

    def order_by(self, *orders: Order) -> Query[T]:
        return replace(self, orders=self.orders + orders)

    def limit(self, value: int) -> Query[T]:
        if value < 0:
            raise ValueError("limit must be non-negative")
        return replace(self, page_limit=value)

    def offset(self, value: int) -> Query[T]:
        if value < 0:
            raise ValueError("offset must be non-negative")
        return replace(self, page_offset=value)

    def group_by(self, *fields: str) -> Aggregate[T]:
        return Aggregate(self.model, self.predicates, fields)

    def _wire(self) -> dict[str, Any]:
        query: dict[str, Any] = {"sort": [item._wire() for item in self.orders]}
        if self.predicates:
            filters = [item._wire() for item in self.predicates]
            query["filter"] = filters[0] if len(filters) == 1 else {"op": "and", "args": filters}
        if self.page_limit is not None:
            query["limit"] = self.page_limit
        if self.page_offset is not None:
            query["offset"] = self.page_offset
        return query


def query(model: type[T]) -> Query[T]:
    return Query(model)


@dataclass(frozen=True, slots=True)
class Get(Generic[T]):
    model: type[T]
    key: Any


def get(model: type[T], key: Any) -> Get[T]:
    return Get(model, key)


@dataclass(frozen=True, slots=True)
class Create(Generic[T]):
    value: T


def create(value: T) -> Create[T]:
    return Create(value)


@dataclass(frozen=True, slots=True)
class Update(Generic[T]):
    model: type[T]
    key: Any
    changes: Mapping[str, Any]
    preconditions: tuple[Expr[bool], ...] = ()
    revision: str | None = None

    def require(self, predicate: Expr[bool]) -> Update[T]:
        return replace(self, preconditions=self.preconditions + (predicate,))

    def if_revision(self, revision: str) -> Update[T]:
        return replace(self, revision=revision)


def update(model: type[T], key: Any, **changes: Any) -> Update[T]:
    if not changes:
        raise ValueError("update requires at least one changed field")
    return Update(model, key, MappingProxyType(dict(changes)))


@dataclass(frozen=True, slots=True)
class Delete(Generic[T]):
    model: type[T]
    key: Any
    preconditions: tuple[Expr[bool], ...] = ()
    revision: str | None = None

    def require(self, predicate: Expr[bool]) -> Delete[T]:
        return replace(self, preconditions=self.preconditions + (predicate,))

    def if_revision(self, revision: str) -> Delete[T]:
        return replace(self, revision=revision)


def delete(model: type[T], key: Any) -> Delete[T]:
    return Delete(model, key)


@dataclass(frozen=True, slots=True)
class Upsert(Generic[T]):
    value: T
    preconditions: tuple[Expr[bool], ...] = ()
    revision: str | None = None

    def require(self, predicate: Expr[bool]) -> Upsert[T]:
        return replace(self, preconditions=self.preconditions + (predicate,))

    def if_revision(self, revision: str) -> Upsert[T]:
        return replace(self, revision=revision)


def upsert(value: T) -> Upsert[T]:
    return Upsert(value)


MutationPlan = Create[Any] | Update[Any] | Delete[Any] | Upsert[Any]


@dataclass(frozen=True, slots=True)
class Batch:
    items: tuple[MutationPlan, ...]


def batch(*items: MutationPlan) -> Batch:
    if not items:
        raise ValueError("batch requires at least one mutation")
    return Batch(items)


@dataclass(frozen=True, slots=True)
class Traverse(Generic[T]):
    link: LinkDefinition[Any, T]
    source_key: Any
    predicates: tuple[Expr[bool], ...] = ()
    orders: tuple[Order, ...] = ()
    page_limit: int | None = None

    def where(self, *predicates: Expr[bool]) -> Traverse[T]:
        return replace(self, predicates=self.predicates + predicates)

    def order_by(self, *orders: Order) -> Traverse[T]:
        return replace(self, orders=self.orders + orders)

    def limit(self, value: int) -> Traverse[T]:
        return replace(self, page_limit=value)


def traverse(link: LinkDefinition[S, T], source_key: Any) -> Traverse[T]:
    return Traverse(link, source_key)


@dataclass(frozen=True, slots=True)
class Aggregate(Generic[T]):
    model: type[T]
    predicates: tuple[Expr[bool], ...]
    groups: tuple[str, ...]
    aggregations: tuple[tuple[str, AggregateExpr], ...] = ()

    def agg(self, **expressions: AggregateExpr) -> Aggregate[T]:
        return replace(self, aggregations=self.aggregations + tuple(expressions.items()))

    def _wire(self) -> dict[str, Any]:
        result: dict[str, Any] = {"group_by": list(self.groups),
                                  "aggregations": [expr._wire(alias) for alias, expr in self.aggregations]}
        if self.predicates:
            filters = [item._wire() for item in self.predicates]
            result["filter"] = filters[0] if len(filters) == 1 else {"op": "and", "args": filters}
        return result
