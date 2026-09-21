"""Pure policy request and decision values."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping

from .context import Context
from .expr import Expr


@dataclass(frozen=True, slots=True)
class Request:
    context: Context
    operation: str
    resource_kind: str
    resource: Any
    input: Any = None

    @property
    def principal(self) -> str:
        return self.context.principal

    def claim(self, name: str, default: Any = None) -> Any:
        return self.context.claims.get(name, default)


@dataclass(frozen=True, slots=True)
class Decision:
    allowed: bool
    reason: str | None = None
    where: Expr[bool] | None = None
    redact: tuple[str, ...] = ()
    require_approval: bool = False
    constraints: tuple[Expr[bool], ...] = ()
    transformations: Mapping[str, Any] | None = None
    obligations: Mapping[str, Any] | None = None

    def _wire(self) -> dict[str, Any]:
        return {
            "allow": self.allowed, "reason": self.reason,
            "row_filter": self.where._wire() if self.where is not None else None,
            "redactions": list(self.redact), "require_approval": self.require_approval,
            "input_constraints": [item._wire() for item in self.constraints],
            "transformations": dict(self.transformations or {}),
            "obligations": dict(self.obligations or {}),
        }


def allow(*, where: Expr[bool] | None = None, redact: tuple[str, ...] = (),
          require_approval: bool = False, constraints: tuple[Expr[bool], ...] = (),
          transformations: Mapping[str, Any] | None = None,
          obligations: Mapping[str, Any] | None = None) -> Decision:
    return Decision(True, where=where, redact=redact, require_approval=require_approval,
                    constraints=constraints, transformations=transformations, obligations=obligations)


def deny(reason: str = "denied by policy") -> Decision:
    return Decision(False, reason=reason)


def allow_all(_request: Request) -> Decision:
    return allow()


def deny_all(_request: Request) -> Decision:
    return deny()
