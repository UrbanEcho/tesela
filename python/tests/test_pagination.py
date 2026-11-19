"""Keyset cursor walks through the Python facade."""
from __future__ import annotations

from typing import Any, Mapping

import pytest
import tesela as ts


@ts.object(key="id")
class Row:
    id: str
    score: int | None = None


def ctx() -> ts.Context:
    return ts.context("test-agent", request_id="request-1")


def seeded(backend: Any | None = None) -> ts.Runtime:
    rt = ts.Runtime({Row: backend or ts.MemoryBackend()}, policy=ts.policy.allow_all)
    for index in range(23):
        rt.run(ts.create(Row(f"r{index:02d}", None if index % 7 == 0 else index % 4)), ctx())
    return rt


def walk(rt: ts.Runtime, plan: ts.Query[Row]) -> list[Row]:
    rows: list[Row] = []
    while True:
        page = rt.run(plan, ctx())
        assert 0 < len(page.items) <= (plan.page_limit or 1000)
        rows.extend(page.items)
        if page.next_cursor is None:
            return rows
        plan = plan.after(page.next_cursor)


def test_cursor_walk_visits_every_row_exactly_once() -> None:
    rt = seeded()
    for plan in (ts.query(Row).limit(5),
                 ts.query(Row).order_by(ts.col("score").asc()).limit(4),
                 ts.query(Row).order_by(ts.col("score").desc()).limit(3)):
        rows = walk(rt, plan)
        assert sorted(row.id for row in rows) == [f"r{index:02d}" for index in range(23)]
        assert len({row.id for row in rows}) == 23
    unsorted = walk(rt, ts.query(Row).limit(5))
    assert [row.id for row in unsorted] == sorted(row.id for row in unsorted)


def test_cursor_from_another_sort_is_rejected() -> None:
    rt = seeded()
    page = rt.run(ts.query(Row).order_by(ts.col("score").asc()).limit(2), ctx())
    assert page.next_cursor is not None
    with pytest.raises(ts.Error, match="different query"):
        rt.run(ts.query(Row).order_by(ts.col("score").desc()).limit(2).after(page.next_cursor), ctx())
    with pytest.raises(ts.Error):
        rt.run(ts.query(Row).limit(2).after("not-a-cursor"), ctx())


def test_limit_zero_is_rejected() -> None:
    rt = seeded()
    with pytest.raises(ValueError, match="limit"):
        ts.query(Row).limit(0)
    with pytest.raises(ts.Error):
        rt.run(ts.Query(Row, page_limit=0), ctx())


def test_offset_is_not_part_of_the_facade_or_the_wire() -> None:
    assert not hasattr(ts.query(Row), "offset")
    assert "offset" not in ts.query(Row).limit(2).after("00")._wire()
    native: Any = ts.Runtime({Row: ts.MemoryBackend()}, policy=ts.policy.allow_all)._native
    with pytest.raises(Exception, match="offset"):
        native.search(ctx()._wire(), "row", {"sort": [], "offset": 3})


def _present(value: Any) -> bool:
    return value is not None


def _matches(filter_: Mapping[str, Any], row: Mapping[str, Any]) -> bool:
    op = filter_["op"]
    if op == "and":
        return all(_matches(arg, row) for arg in filter_["args"])
    if op == "or":
        return any(_matches(arg, row) for arg in filter_["args"])
    value = row.get(filter_["field"])
    if op == "is_null":
        return not _present(value)
    if op == "is_not_null":
        return _present(value)
    if not _present(value):
        return False
    if op == "eq":
        return value == filter_["value"]
    if op == "gt":
        return value > filter_["value"]
    if op == "lt":
        return value < filter_["value"]
    raise AssertionError(f"unexpected keyset operator {op}")


class KeysetNative(ts.backend.Native[Row]):
    """Honours filter, sort (primary-key tiebreak) and limit, as documented."""

    def __init__(self, rows: list[Row]) -> None:
        self.rows, self.queries = rows, []

    def search(self, context: ts.Context, query: Mapping[str, Any]) -> list[Row]:
        self.queries.append(query)
        rows = [row for row in self.rows if "filter" not in query or _matches(query["filter"], {"id": row.id, "score": row.score})]
        rows.sort(key=lambda row: row.id)
        for sort in reversed(query["sort"]):
            descending = sort["direction"] == "desc"
            # Missing sorts first ascending and last descending: one key, reversed.
            rows.sort(key=lambda row: (row.score is not None, row.score or 0), reverse=descending)
        return rows[: query["limit"]]


def test_python_native_is_paged_through_the_keyset_filter() -> None:
    rows = [Row(f"r{index:02d}", None if index % 7 == 0 else index % 4) for index in range(23)]
    native = KeysetNative(rows)
    rt = ts.Runtime({Row: native}, policy=ts.policy.allow_all)
    for plan in (ts.query(Row).limit(5), ts.query(Row).order_by(ts.col("score").asc()).limit(4),
                 ts.query(Row).order_by(ts.col("score").desc()).limit(6)):
        native.queries.clear()
        seen = walk(rt, plan)
        assert sorted(row.id for row in seen) == sorted(row.id for row in rows)
        assert len(seen) == len(rows)
        assert any("filter" in query for query in native.queries[1:])
        assert all(query["limit"] == (plan.page_limit or 0) + 1 for query in native.queries)
