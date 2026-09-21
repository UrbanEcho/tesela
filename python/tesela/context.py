"""Explicit immutable execution authority and resource limits."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone
from threading import Event
from types import MappingProxyType
from typing import Any, Mapping


@dataclass(frozen=True, slots=True)
class Budget:
    calls: int | None = None
    rows: int | None = None
    bytes: int | None = None
    external_cost: int | None = None
    wall_time_seconds: float | None = None

    def __post_init__(self) -> None:
        for name in ("calls", "rows", "bytes", "external_cost"):
            value = getattr(self, name)
            if value is not None and value < 0:
                raise ValueError(f"{name} must be non-negative")
        if self.wall_time_seconds is not None and self.wall_time_seconds < 0:
            raise ValueError("wall_time_seconds must be non-negative")


def budget(*, calls: int | None = None, rows: int | None = None, bytes: int | None = None,
           external_cost: int | None = None, wall_time_seconds: float | None = None) -> Budget:
    return Budget(calls, rows, bytes, external_cost, wall_time_seconds)


class Cancellation:
    """Thread-safe cancellation handle shared by immutable contexts."""

    def __init__(self) -> None:
        self._event = Event()

    def cancel(self) -> None:
        self._event.set()

    @property
    def cancelled(self) -> bool:
        return self._event.is_set()


@dataclass(frozen=True, slots=True)
class Context:
    principal: str
    request_id: str
    claims: Mapping[str, Any] = field(default_factory=dict)
    approvals: tuple[str, ...] = ()
    authority: tuple[Mapping[str, Any], ...] = ()
    deadline: datetime | None = None
    budget: Budget | None = None
    cancellation: Cancellation | None = None

    def __post_init__(self) -> None:
        if not self.principal:
            raise ValueError("principal must not be empty")
        if not self.request_id:
            raise ValueError("request_id must not be empty")
        object.__setattr__(self, "claims", MappingProxyType(dict(self.claims)))
        object.__setattr__(self, "approvals", tuple(self.approvals))
        object.__setattr__(self, "authority", tuple(MappingProxyType(dict(item)) for item in self.authority))

    def _wire(self) -> dict[str, Any]:
        remaining = None
        if self.deadline is not None:
            deadline = self.deadline
            if deadline.tzinfo is None:
                raise ValueError("deadline must be timezone-aware")
            remaining = max(0.0, (deadline - datetime.now(timezone.utc)).total_seconds())
        return {
            "principal": {"id": self.principal, "claims": dict(self.claims)},
            "request_id": self.request_id,
            "approvals": list(self.approvals),
            "authority": list(self.authority),
            "deadline_seconds": remaining,
            "budget": None if self.budget is None else {
                "calls": self.budget.calls, "rows": self.budget.rows, "bytes": self.budget.bytes,
                "external_cost": self.budget.external_cost,
                "wall_time_seconds": self.budget.wall_time_seconds,
            },
            "cancelled": self.cancellation.cancelled if self.cancellation else False,
        }


def context(principal: str, *, request_id: str, claims: Mapping[str, Any] | None = None,
            approvals: tuple[str, ...] = (), authority: tuple[Mapping[str, Any], ...] = (),
            deadline: datetime | None = None, budget: Budget | None = None,
            cancellation: Cancellation | None = None) -> Context:
    return Context(principal, request_id, dict(claims or {}), approvals, authority,
                   deadline, budget, cancellation)
