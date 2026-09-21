"""Typed runtime facade over the native extension."""

from __future__ import annotations

import datetime as _datetime
import decimal as _decimal
import enum as _enum
import types as _types
import uuid as _uuid
from typing import Annotated, Any, cast, get_args, get_origin, get_type_hints

from . import _native
from .model import Spec

MemoryBackend = _native.NativeMemoryBackend

class _BackendAdapter:
    def __init__(self, backend: Any):
        self.backend = backend

    def __getattr__(self, method: str):
        target = getattr(self.backend, method)
        def call(payload: dict[str, Any]):
            payload = dict(payload)
            payload.pop("expected_revision", None) if payload.get("expected_revision") is None else None
            return target(**payload)
        return call


class _PolicyAdapter:
    def __init__(self, policy: Any): self.policy = policy
    def evaluate(self, payload: dict[str, Any]): return self.policy.evaluate(payload["request"])


class _ActionAdapter:
    def __init__(self, action: Any): self.action = action
    def execute(self, payload: dict[str, Any]): return self.action(payload["request"])


class _LinkAdapter:
    def __init__(self, resolver: Any): self.resolver = resolver
    def linked_keys(self, payload: dict[str, Any]):
        return self.resolver(payload["link"], payload["target"], payload["source_key"])


def _coerce(annotation: Any, value: Any) -> Any:
    if value is None:
        return None
    origin = get_origin(annotation)
    if origin in (_types.UnionType, __import__("typing").Union):
        annotation = next((item for item in get_args(annotation) if item is not type(None)), annotation)
        origin = get_origin(annotation)
    if origin is Annotated:
        annotation = get_args(annotation)[0]
        origin = get_origin(annotation)
    if origin is list:
        item = get_args(annotation)[0] if get_args(annotation) else Any
        return [_coerce(item, element) for element in value]
    if isinstance(annotation, type) and issubclass(annotation, _enum.Enum):
        return annotation(value)
    if annotation is _uuid.UUID:
        return _uuid.UUID(value)
    if annotation is _decimal.Decimal:
        return _decimal.Decimal(value)
    if annotation is _datetime.datetime:
        return _datetime.datetime.fromisoformat(value)
    if annotation is _datetime.date:
        return _datetime.date.fromisoformat(value)
    return value


class QuerySet:
    """Lazy typed query assembled before one native call."""

    def __init__(self, runtime: "Runtime", model: type, context: dict[str, Any]):
        self.runtime, self.model, self.context = runtime, model, context
        self.query: dict[str, Any] = {}

    def where(self, predicate: dict[str, Any]) -> "QuerySet":
        existing = self.query.get("filter")
        self.query["filter"] = predicate if existing is None else {"op": "and", "args": [existing, predicate]}
        return self

    def take(self, limit: int) -> "QuerySet":
        self.query["limit"] = limit
        return self

    def load(self) -> list[Any]:
        return self.runtime.search(self.model, self.query, context=self.context)["records"]


class Runtime:
    """Typed Python runtime over direct PyO3 values."""

    def __init__(self, spec: Spec, *, objects: dict[type | str, Any], policy: Any, links: dict[str, Any] | None = None, actions: dict[type | str, Any] | None = None, max_query_limit: int = 1000):
        if policy is None:
            raise TypeError("policy is required")
        self.spec = spec
        self.models = {name: model for model, name in spec.models.items()}
        native_objects: dict[str, object] = {self._name(model): backend if isinstance(backend, MemoryBackend) else _BackendAdapter(backend) for model, backend in objects.items()}
        native_actions: dict[str, object] = {self._name(action): _ActionAdapter(handler) for action, handler in (actions or {}).items()}
        native_links: dict[str, object] = {name: _LinkAdapter(handler) for name, handler in (links or {}).items()}
        native_policy = policy if policy == "allow_all" else _PolicyAdapter(policy)
        self._native = _native.NativeRuntime(spec.to_dict(), native_objects, native_policy, links=native_links, actions=native_actions, max_query_limit=max_query_limit)

    def _name(self, model: type | str) -> str:
        return cast(str, getattr(model, "__tesela_name__", model))

    def _context(self, context: dict[str, Any] | None) -> dict[str, Any]:
        return context or {"principal": {"id": "local"}, "request_id": "python"}

    def _materialize(self, model: type | str, record: dict[str, Any]) -> Any:
        if not isinstance(model, type):
            return record
        hints = getattr(model, "__tesela_hints__", None) or get_type_hints(model, include_extras=True)
        values = {name: _coerce(hints.get(name, Any), value) for name, value in record["values"].items()}
        try:
            return model(**values)
        except TypeError:
            instance = cast(Any, object.__new__(model))
            for key, value in values.items():
                setattr(instance, key, value)
            setattr(instance, "__tesela_revision__", record.get("revision"))
            return instance

    def objects(self, model: type, *, context: dict[str, Any] | None = None) -> QuerySet:
        return QuerySet(self, model, self._context(context))

    def search(self, model: type | str, query: dict[str, Any] | None = None, *, context: dict[str, Any] | None = None) -> dict[str, Any]:
        execution = self._native.search(self._context(context), self._name(model), query or {})
        page = execution["output"]
        page["records"] = [self._materialize(model, record) for record in page["records"]]
        page["receipt"] = execution["receipt"]
        return page

    def get(self, model: type | str, key: Any, *, context: dict[str, Any] | None = None) -> Any:
        execution = self._native.get(self._context(context), self._name(model), key)
        return self._materialize(model, execution["output"])

    def mutate(self, model: type | str, mutation: dict[str, Any], *, context: dict[str, Any] | None = None) -> dict[str, Any]:
        return self._native.mutate(self._context(context), self._name(model), mutation)

    def aggregate(self, model: type | str, query: dict[str, Any] | None = None, *, context: dict[str, Any] | None = None) -> dict[str, Any]:
        return self._native.aggregate(self._context(context), self._name(model), query or {})

    def traverse(self, link: str, query: dict[str, Any], *, context: dict[str, Any] | None = None) -> dict[str, Any]:
        return self._native.traverse(self._context(context), link, query)

    def execute_action(self, action: type | str, input: dict[str, Any], *, context: dict[str, Any] | None = None, expected_revision: str | None = None) -> dict[str, Any]:
        request: dict[str, Any] = {"action": self._name(action), "input": input, "expected_revision": expected_revision, "preconditions": []}
        return self._native.execute_action(self._context(context), request)

    def capabilities(self, task: str, *, context: dict[str, Any] | None = None) -> dict[str, Any]:
        return self._native.capabilities(self._context(context), task)

