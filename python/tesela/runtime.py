"""The sole execution boundary for immutable Tesela plans."""
from __future__ import annotations

import dataclasses
import datetime as _datetime
import decimal as _decimal
import enum as _enum
import types as _types
import uuid as _uuid
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from typing import (Annotated, Any, Mapping, TypeVar, cast, get_args, get_origin,
                    get_type_hints, overload)

from . import _native
from .backend import Changes, Native, Repository
from .context import Budget, Cancellation, Context
from .model import LinkDefinition, _name, encode_model, model_definition
from .plan import (Aggregate, AggregationResult, Batch, BatchResult, Create, Delete,
                   Execution, Get, Page, Query, Traverse, Update, Upsert)
from .policy import Decision, Request

MemoryBackend = _native.NativeMemoryBackend
T = TypeVar("T")


@dataclass(frozen=True, slots=True)
class Schema:
    objects: tuple[Mapping[str, Any], ...]
    links: tuple[Mapping[str, Any], ...]
    actions: tuple[Mapping[str, Any], ...]


def _context(value: Mapping[str, Any]) -> Context:
    principal = cast(Mapping[str, Any], value.get("principal", {}))
    raw_budget = cast(Mapping[str, Any], value.get("budget", {}))
    seconds = cast(float | None, value.get("deadline_seconds"))
    cancellation = Cancellation()
    if value.get("cancelled"):
        cancellation.cancel()
    return Context(str(principal.get("id", "")), str(value.get("request_id", "")),
                   cast(Mapping[str, Any], principal.get("claims", {})),
                   tuple(cast(list[str], value.get("approvals", []))),
                   tuple(cast(list[Mapping[str, Any]], value.get("authority", []))),
                   None if seconds is None else datetime.now(timezone.utc) + timedelta(seconds=seconds),
                   Budget(raw_budget.get("calls"), raw_budget.get("rows"), raw_budget.get("bytes"),
                          raw_budget.get("external_cost"), raw_budget.get("wall_time_seconds")),
                   cancellation)


def _record(model: type, value: Any, revision: str | None = None) -> dict[str, Any]:
    values = encode_model(value)
    return {"primary_key": values[getattr(model, "__tesela_key__")],
            "values": values, "revision": revision}


class _BackendAdapter:
    def __init__(self, model: type, implementation: Repository[Any] | Native[Any]):
        self.model, self.implementation = model, implementation
        self.__tesela_capabilities__ = (
            ("search",) + (("aggregate",) if type(implementation).aggregate is not Native.aggregate else ())
            if isinstance(implementation, Native)
            else tuple(name for name in ("scan", "get", "create", "update", "delete")
                       if getattr(implementation, name) is not None))

    def scan(self, payload: Mapping[str, Any]) -> list[dict[str, Any]]:
        callback = cast(Repository[Any], self.implementation).scan
        assert callback is not None
        return [_record(self.model, item) for item in callback(_context(payload["context"]))]

    def search(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        callback = cast(Native[Any], self.implementation).search
        items = callback(_context(payload["context"]), cast(Mapping[str, Any], payload["query"]))
        return {"records": [_record(self.model, item) for item in items], "next_cursor": None}

    def aggregate(self, payload: Mapping[str, Any]) -> Mapping[str, Any]:
        callback = cast(Native[Any], self.implementation).aggregate
        return callback(_context(payload["context"]), cast(Mapping[str, Any], payload["query"]))

    def get(self, payload: Mapping[str, Any]) -> dict[str, Any] | None:
        callback = cast(Repository[Any], self.implementation).get
        assert callback is not None
        value = callback(_context(payload["context"]), payload["primary_key"])
        return None if value is None else _record(self.model, value)

    def create(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        callback = cast(Repository[Any], self.implementation).create
        assert callback is not None
        stored = callback(_context(payload["context"]), self.model(**payload["values"]))
        return {"record": _record(self.model, stored), "rows_affected": 1}

    def update(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        callback = cast(Repository[Any], self.implementation).update
        assert callback is not None
        stored = callback(_context(payload["context"]), payload["primary_key"],
                          Changes(cast(Mapping[str, Any], payload["values"])))
        return {"record": _record(self.model, stored), "rows_affected": 1}

    def delete(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        callback = cast(Repository[Any], self.implementation).delete
        assert callback is not None
        callback(_context(payload["context"]), payload["primary_key"])
        return {"record": None, "rows_affected": 1}


class _PolicyAdapter:
    def __init__(self, function: Any, resources: Mapping[str, Any]):
        self.function, self.resources = function, resources

    def evaluate(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        raw = cast(Mapping[str, Any], payload["request"])
        decision = self.function(Request(
            _context(cast(Mapping[str, Any], raw["context"])), str(raw["operation"]),
            str(raw["resource_kind"]), self.resources.get(str(raw["resource"]), raw["resource"]),
            raw.get("input")))
        if not isinstance(decision, Decision):
            raise TypeError("policy functions must return tesela.policy.Decision")
        return decision._wire()


class _ActionAdapter:
    def __init__(self, command: type, handler: Any):
        self.command, self.handler = command, handler

    def execute(self, payload: Mapping[str, Any]) -> dict[str, Any]:
        request = cast(Mapping[str, Any], payload["request"])
        command = self.command(**cast(Mapping[str, Any], request.get("input", {})))
        output = self.handler(_context(cast(Mapping[str, Any], payload["context"])), command)
        return {"status": "success",
                "output": encode_model(output) if dataclasses.is_dataclass(output) else output,
                "run_id": request.get("run_id")}


class _LinkAdapter:
    def __init__(self, resolver: Any): self.resolver = resolver
    def linked_keys(self, payload: Mapping[str, Any]) -> list[Any]:
        return list(self.resolver(_context(cast(Mapping[str, Any], payload["context"])),
                                  payload["source_key"]))


def _coerce(annotation: Any, value: Any) -> Any:
    if value is None: return None
    origin = get_origin(annotation)
    if origin in (_types.UnionType, __import__("typing").Union):
        annotation = next((item for item in get_args(annotation) if item is not type(None)), annotation)
        origin = get_origin(annotation)
    if origin is Annotated:
        annotation, origin = get_args(annotation)[0], get_origin(get_args(annotation)[0])
    if origin is list:
        item = get_args(annotation)[0] if get_args(annotation) else Any
        return [_coerce(item, element) for element in value]
    if isinstance(annotation, type) and issubclass(annotation, _enum.Enum): return annotation(value)
    if annotation is _uuid.UUID: return _uuid.UUID(value)
    if annotation is _decimal.Decimal: return _decimal.Decimal(value)
    if annotation is _datetime.datetime: return _datetime.datetime.fromisoformat(value)
    if annotation is _datetime.date: return _datetime.date.fromisoformat(value)
    return value


def _materialize(model: type, record: Mapping[str, Any]) -> Any:
    hints = getattr(model, "__tesela_hints__", None) or get_type_hints(model, include_extras=True)
    values = {name: _coerce(hints.get(name, Any), value)
              for name, value in cast(Mapping[str, Any], record["values"]).items()}
    return model(**values)


class Runtime:
    """Compiled immutable ontology plus host-owned bindings."""
    def __init__(self, objects: Mapping[type, Any], *, policy: Any,
                 links: Mapping[LinkDefinition[Any, Any], Any] | None = None,
                 actions: Mapping[type, Any] | None = None, max_query_limit: int = 1000):
        if not callable(policy): raise TypeError("policy must be a Request -> Decision function")
        self._models = {_name(model): model for model in objects}
        self._links, self._actions = tuple((links or {}).keys()), tuple((actions or {}).keys())
        resources: dict[str, Any] = dict(self._models)
        resources.update((link.name, link) for link in self._links)
        resources.update((_name(action), action) for action in self._actions)
        traits = {getattr(item, "__tesela_definition__")["api_name"]: getattr(item, "__tesela_definition__")
                  for model in objects for item in getattr(model, "__tesela_traits__", ())}
        spec = {"version": "tesela.spec.v1",
                "traits": list(traits.values()),
                "object_types": [dict(model_definition(model)) for model in objects],
                "link_types": [link._wire() for link in self._links],
                "actions": [dict(getattr(action, "__tesela_action__")) for action in self._actions]}
        native_objects = {_name(model): implementation if isinstance(implementation, MemoryBackend)
                          else _BackendAdapter(model, implementation)
                          for model, implementation in objects.items()}
        self._native = _native.NativeRuntime(
            spec, native_objects, _PolicyAdapter(policy, resources),
            links={link.name: _LinkAdapter(resolver) for link, resolver in (links or {}).items()},
            actions={_name(action): _ActionAdapter(action, handler) for action, handler in (actions or {}).items()},
            max_query_limit=max_query_limit)

    @overload
    def run(self, plan: Query[T], context: Context) -> Page[T]: ...
    @overload
    def run(self, plan: Get[T], context: Context) -> T | None: ...
    @overload
    def run(self, plan: Create[T] | Update[T] | Upsert[T], context: Context) -> T: ...
    @overload
    def run(self, plan: Delete[T], context: Context) -> None: ...
    @overload
    def run(self, plan: Batch, context: Context) -> BatchResult: ...
    @overload
    def run(self, plan: Traverse[T], context: Context) -> Page[T]: ...
    @overload
    def run(self, plan: Aggregate[T], context: Context) -> AggregationResult: ...
    @overload
    def run(self, plan: Any, context: Context) -> Any: ...
    def run(self, plan: Any, context: Context) -> Any:
        return self.execute(plan, context).value

    @overload
    def execute(self, plan: Query[T], context: Context) -> Execution[Page[T]]: ...
    @overload
    def execute(self, plan: Get[T], context: Context) -> Execution[T | None]: ...
    @overload
    def execute(self, plan: Create[T] | Update[T] | Upsert[T], context: Context) -> Execution[T]: ...
    @overload
    def execute(self, plan: Delete[T], context: Context) -> Execution[None]: ...
    @overload
    def execute(self, plan: Any, context: Context) -> Execution[Any]: ...
    def execute(self, plan: Any, context: Context) -> Execution[Any]:
        if not isinstance(context, Context): raise TypeError("execution requires an explicit tesela.Context")
        native, value, revision = self._execute(plan, context), None, None
        output = native["output"]
        if isinstance(plan, Query):
            value = Page(tuple(_materialize(plan.model, item) for item in output["records"]), output.get("next_cursor"))
        elif isinstance(plan, Get):
            value = None if output is None else _materialize(plan.model, output)
            revision = None if output is None else output.get("revision")
        elif isinstance(plan, (Create, Update, Upsert)):
            model = type(plan.value) if isinstance(plan, (Create, Upsert)) else plan.model
            record = output.get("record")
            if record is None: raise RuntimeError("mutation backend did not return the stored record")
            value, revision = _materialize(model, record), record.get("revision")
        elif isinstance(plan, Delete): value = None
        elif isinstance(plan, Batch): value = BatchResult(output.get("rows_affected"))
        elif isinstance(plan, Traverse):
            value = Page(tuple(_materialize(plan.link.target, item) for item in output["records"]), output.get("next_cursor"))
        elif isinstance(plan, Aggregate): value = AggregationResult(tuple(output["groups"]))
        elif hasattr(type(plan), "__tesela_action__"):
            value = self._materialize_action(type(plan), output.get("output"))
        else: raise TypeError(f"unsupported plan {type(plan).__name__}")
        return Execution(value, native["receipt"], revision)

    def _execute(self, plan: Any, context: Context) -> Mapping[str, Any]:
        wire = context._wire()
        if isinstance(plan, Query): return self._native.search(wire, _name(plan.model), plan._wire())
        if isinstance(plan, Get): return self._native.lookup(wire, _name(plan.model), plan.key)
        if isinstance(plan, (Create, Update, Delete, Upsert, Batch)):
            model, mutation = self._mutation(plan)
            return self._native.mutate(wire, _name(model), dict(mutation))
        if isinstance(plan, Traverse):
            query = Query(plan.link.target, plan.predicates, plan.orders, plan.page_limit)._wire()
            query["source_pk"] = plan.source_key
            return self._native.traverse(wire, plan.link.name, query)
        if isinstance(plan, Aggregate): return self._native.aggregate(wire, _name(plan.model), plan._wire())
        definition = getattr(type(plan), "__tesela_action__", None)
        if definition is not None:
            return self._native.execute_action(wire, {"action": definition["api_name"],
                "input": encode_model(plan), "preconditions": []})
        raise TypeError(f"unsupported plan {type(plan).__name__}")

    def _mutation(self, plan: Any) -> tuple[type, Mapping[str, Any]]:
        if isinstance(plan, Create): return type(plan.value), {"create": {"values": encode_model(plan.value)}}
        if isinstance(plan, Update): return plan.model, {"update": {"primary_key": plan.key, "values": dict(plan.changes), "expected_revision": plan.revision, "preconditions": [item._wire() for item in plan.preconditions]}}
        if isinstance(plan, Delete): return plan.model, {"delete": {"primary_key": plan.key, "expected_revision": plan.revision, "preconditions": [item._wire() for item in plan.preconditions]}}
        if isinstance(plan, Upsert): return type(plan.value), {"upsert": {"values": encode_model(plan.value), "expected_revision": plan.revision, "preconditions": [item._wire() for item in plan.preconditions]}}
        items = [self._mutation(item) for item in cast(Batch, plan).items]
        model = items[0][0]
        if any(item_model is not model for item_model, _ in items): raise ValueError("atomic batches must target one object type")
        return model, {"batch": {"items": [item for _, item in items]}}

    def _materialize_action(self, action: type, output: Any) -> Any:
        result_type = getattr(action, "__tesela_returns__")
        if dataclasses.is_dataclass(result_type) and isinstance(output, Mapping):
            hints = get_type_hints(result_type, include_extras=True)
            return cast(Any, result_type)(**{name: _coerce(hints.get(name, Any), value) for name, value in output.items()})
        return _coerce(result_type, output)

    def explain(self, plan: Query[Any], context: Context) -> Mapping[str, Any]:
        if not isinstance(plan, Query): raise TypeError("explain currently accepts query plans")
        return cast(Mapping[str, Any], self._native.explain(context._wire(), _name(plan.model), plan._wire()))

    def capabilities(self, context: Context, task: str = "") -> Mapping[str, Any]:
        return cast(Mapping[str, Any], self._native.capabilities(context._wire(), task))

    def invoke(self, context: Context, invocation: Mapping[str, Any]) -> Mapping[str, Any]:
        if invocation.get("retain_results"):
            raise ValueError("retain_results is not available in the Python facade: handle records stay on the Rust side")
        return cast(Mapping[str, Any], self._native.invoke(context._wire(), dict(invocation)))

    def schema(self) -> Schema:
        spec = self._native.spec()
        return Schema(tuple(spec.get("object_types", ())), tuple(spec.get("link_types", ())), tuple(spec.get("actions", ())))
