"""Immutable composable expressions."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Generic, Iterable, TypeVar

T = TypeVar("T")


@dataclass(frozen=True, slots=True, eq=False)
class Expr(Generic[T]):
    _op: str
    _field: str | None = None
    _value: Any = None
    _args: tuple[Expr[Any], ...] = ()

    def _compare(self, op: str, value: Any) -> Expr[bool]:
        if self._op != "col":
            raise TypeError("comparisons require a column expression")
        return Expr(op, self._field, value)

    def __eq__(self, value: object) -> Expr[bool]:  # type: ignore[override]
        return self._compare("eq", value)

    def __ne__(self, value: object) -> Expr[bool]:  # type: ignore[override]
        return self._compare("ne", value)

    def __lt__(self, value: Any) -> Expr[bool]:
        return self._compare("lt", value)

    def __le__(self, value: Any) -> Expr[bool]:
        return self._compare("lte", value)

    def __gt__(self, value: Any) -> Expr[bool]:
        return self._compare("gt", value)

    def __ge__(self, value: Any) -> Expr[bool]:
        return self._compare("gte", value)

    def __and__(self, other: Expr[bool]) -> Expr[bool]:
        return Expr("and", _args=(self._predicate(), other._predicate()))

    def __or__(self, other: Expr[bool]) -> Expr[bool]:
        return Expr("or", _args=(self._predicate(), other._predicate()))

    def __invert__(self) -> Expr[bool]:
        return Expr("not", _args=(self._predicate(),))

    def __bool__(self) -> bool:
        raise TypeError("Tesela expressions use '&', '|', and '~', not Python 'and', 'or', or truth testing")

    def is_null(self) -> Expr[bool]:
        return self._compare("is_null", None)

    def is_not_null(self) -> Expr[bool]:
        return self._compare("is_not_null", None)

    def is_in(self, values: Iterable[Any]) -> Expr[bool]:
        if self._op != "col":
            raise TypeError("is_in requires a column expression")
        return Expr("in", self._field, tuple(values))

    def starts_with(self, value: str) -> Expr[bool]:
        return self._compare("starts_with", value)

    def ends_with(self, value: str) -> Expr[bool]:
        return self._compare("ends_with", value)

    def contains(self, value: str) -> Expr[bool]:
        return self._compare("contains", value)

    def asc(self) -> Order:
        return Order(self._column(), "asc")

    def desc(self) -> Order:
        return Order(self._column(), "desc")

    def sum(self) -> AggregateExpr:
        return AggregateExpr("sum", self._column())

    def mean(self) -> AggregateExpr:
        return AggregateExpr("avg", self._column())

    def min(self) -> AggregateExpr:
        return AggregateExpr("min", self._column())

    def max(self) -> AggregateExpr:
        return AggregateExpr("max", self._column())

    def _column(self) -> str:
        if self._op != "col" or self._field is None:
            raise TypeError("operation requires a column expression")
        return self._field

    def _predicate(self) -> Expr[bool]:
        return self._compare("eq", True) if self._op == "col" else self  # type: ignore[return-value]

    def _wire(self) -> dict[str, Any]:
        predicate = self._predicate()
        if predicate._op in {"and", "or", "not"}:
            return {"op": predicate._op, "args": [item._wire() for item in predicate._args]}
        result: dict[str, Any] = {"op": predicate._op, "field": predicate._field}
        if predicate._op == "in":
            result["values"] = list(predicate._value)
        elif predicate._op not in {"is_null", "is_not_null"}:
            result["value"] = predicate._value
        return result


@dataclass(frozen=True, slots=True)
class Order:
    field: str
    direction: str

    def _wire(self) -> dict[str, str]:
        return {"property": self.field, "direction": self.direction}


@dataclass(frozen=True, slots=True)
class AggregateExpr:
    function: str
    field: str | None = None

    def _wire(self, alias: str) -> dict[str, Any]:
        return {"function": self.function, "property": self.field, "alias": alias}


def col(name: str) -> Expr[Any]:
    if not name:
        raise ValueError("column name must not be empty")
    return Expr("col", name)


def lit(value: T) -> Expr[T]:
    return Expr("lit", _value=value)


def count() -> AggregateExpr:
    return AggregateExpr("count")
