"""Semantic declarations and typed query fields."""

from __future__ import annotations

import datetime as _datetime
import decimal as _decimal
import enum as _enum
import inspect as _inspect
import json as _json
import sys as _sys
import types as _types
import uuid as _uuid
from copy import deepcopy
from typing import Annotated, Any, cast, get_args, get_origin, get_type_hints

from . import _native

class Vector:
    """Annotation metadata for a fixed-dimension vector."""

    def __init__(self, dimensions: int):
        if dimensions <= 0:
            raise ValueError("vector dimensions must be positive")
        self.dimensions = dimensions


class Field:
    """Typed expression descriptor installed on declared model classes."""

    def __init__(self, name: str):
        self.name = name

    def _filter(self, op: str, value: Any = None) -> dict[str, Any]:
        result = {"op": op, "field": self.name}
        if value is not None:
            result["value"] = value
        return result

    def __eq__(self, value: Any) -> dict[str, Any]:  # type: ignore[override]
        return self._filter("eq", value)

    def __ne__(self, value: Any) -> dict[str, Any]:  # type: ignore[override]
        return self._filter("ne", value)

    def __lt__(self, value: Any) -> dict[str, Any]:
        return self._filter("lt", value)

    def __le__(self, value: Any) -> dict[str, Any]:
        return self._filter("lte", value)

    def __gt__(self, value: Any) -> dict[str, Any]:
        return self._filter("gt", value)

    def __ge__(self, value: Any) -> dict[str, Any]:
        return self._filter("gte", value)

    def is_null(self) -> dict[str, Any]:
        return self._filter("is_null")

    def is_not_null(self) -> dict[str, Any]:
        return self._filter("is_not_null")

    def starts_with(self, value: str) -> dict[str, Any]:
        return self._filter("starts_with", value)

    def contains(self, value: str) -> dict[str, Any]:
        return self._filter("contains", value)


def _snake(name: str) -> str:
    result = "".join(("_" if index and char.isupper() else "") + char.lower() for index, char in enumerate(name))
    if not result or not result[0].islower() or not all(char.islower() or char.isdigit() or char == "_" for char in result):
        raise ValueError(f"invalid api_name {result!r}")
    return result


def _property_type(annotation: Any) -> tuple[Any, bool, dict[str, Any]]:
    nullable = False
    origin = get_origin(annotation)
    if origin in (_types.UnionType, __import__("typing").Union):
        args = get_args(annotation)
        if type(None) in args:
            nullable = True
            annotation = next(item for item in args if item is not type(None))
            origin = get_origin(annotation)
    metadata: dict[str, Any] = {}
    if origin is Annotated:
        annotation, *annotations = get_args(annotation)
        vector = next((item for item in annotations if isinstance(item, Vector)), None)
        if vector:
            metadata["data_type"] = {"vector": vector.dimensions}
            return annotation, nullable, metadata
        origin = get_origin(annotation)
    if isinstance(annotation, type) and issubclass(annotation, _enum.Enum):
        metadata["data_type"] = "enum"
        metadata["allowed_values"] = [item.value for item in annotation]
    else:
        mapping = {
            str: "string", int: "integer", float: "float", bool: "boolean",
            dict: "json", _uuid.UUID: "uuid", _datetime.date: "date",
            _datetime.datetime: "timestamptz", _decimal.Decimal: "decimal",
        }
        if origin is list:
            metadata["data_type"] = "array"
        elif annotation in mapping:
            metadata["data_type"] = mapping[annotation]
        else:
            raise TypeError(f"unsupported property annotation {annotation!r}")
    return cast(Any, annotation), nullable, metadata


class Spec:
    """Canonical semantic ontology document."""

    def __init__(self, *, data: dict[str, Any] | None = None):
        self._data: dict[str, Any] = deepcopy(data) if data else {"version": "tesela.spec.v1"}
        self.models: dict[type, str] = {}

    def add(self, section: str, item: dict[str, Any]) -> None:
        if section not in {"traits", "object_types", "link_types", "actions"}:
            raise ValueError(f"unknown Spec section {section!r}")
        entries = cast(list[dict[str, Any]], self._data.setdefault(section, []))
        entries[:] = [existing for existing in entries if existing["api_name"] != item["api_name"]]
        entries.append(deepcopy(item))

    def object_type(self, *, primary_key: str, name: str | None = None, **fields: Any):
        frame = _inspect.currentframe()
        localns = dict(frame.f_back.f_locals) if frame is not None and frame.f_back is not None else {}
        def decorate(cls: type) -> type:
            api_name = name or _snake(cls.__name__)
            properties: list[dict[str, Any]] = []
            namespace = {**localns, cls.__name__: cls}
            hints = get_type_hints(cls, globalns=vars(_sys.modules[cls.__module__]), localns=namespace, include_extras=True)
            for property_name, annotation in hints.items():
                _, nullable, definition = _property_type(annotation)
                properties.append({"api_name": property_name, **definition, **({"nullable": True} if nullable else {})})
                setattr(cls, property_name, Field(property_name))
            if primary_key not in {item["api_name"] for item in properties}:
                raise ValueError(f"primary key {primary_key!r} is not annotated")
            self.add("object_types", {"api_name": api_name, "primary_key": primary_key, "properties": properties, **fields})
            self.models[cls] = api_name
            setattr(cls, "__tesela_name__", api_name)
            setattr(cls, "__tesela_hints__", hints)
            return cls
        return decorate

    def action(self, *, subject: type | str | None = None, name: str | None = None, **fields: Any):
        """Declare an action class. Functions are rejected because their body would be ignored."""
        frame = _inspect.currentframe()
        localns = dict(frame.f_back.f_locals) if frame is not None and frame.f_back is not None else {}
        def decorate(cls: Any) -> type:
            if not isinstance(cls, type):
                raise TypeError("@spec.action decorates a declaration class; bind executable callables on Runtime")
            action_name = name or _snake(cls.__name__)
            properties: dict[str, dict[str, Any]] = {}
            required: list[str] = []
            hints = get_type_hints(cls, globalns=vars(_sys.modules[cls.__module__]), localns={**localns, cls.__name__: cls}, include_extras=True)
            for field_name, annotation in hints.items():
                _, nullable, definition = _property_type(annotation)
                field_schema: dict[str, Any] = {"type": definition["data_type"] if isinstance(definition["data_type"], str) else "array"}
                if "allowed_values" in definition:
                    field_schema["enum"] = definition["allowed_values"]
                properties[field_name] = field_schema
                if not nullable:
                    required.append(field_name)
            subject_name = getattr(subject, "__tesela_name__", subject)
            self.add("actions", {"api_name": action_name, "subject": subject_name, "input_schema": {"type": "object", "properties": properties, "required": required}, **fields})
            setattr(cls, "__tesela_name__", action_name)
            setattr(cls, "__tesela_hints__", hints)
            return cls
        return decorate

    def link(self, *, from_type: type | str, to_type: type | str, cardinality: str = "one_to_many", name: str | None = None, **fields: Any):
        def decorate(cls: type) -> type:
            self.add("link_types", {"api_name": name or _snake(cls.__name__), "from": getattr(from_type, "__tesela_name__", from_type), "to": getattr(to_type, "__tesela_name__", to_type), "cardinality": cardinality, **fields})
            return cls
        return decorate

    def to_dict(self) -> dict[str, Any]:
        return _native.normalize_spec(deepcopy(self._data))

    def to_json(self) -> str:
        """Return canonical JSON for interchange, never for native calls."""
        return _json.dumps(self.to_dict(), separators=(",", ":"), sort_keys=True)

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "Spec":
        return cls(data=_native.normalize_spec(data))

    @classmethod
    def from_json(cls, data: str) -> "Spec":
        return cls.from_dict(_json.loads(data))
