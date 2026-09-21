"""Immutable semantic object, trait, link, and action definitions."""

from __future__ import annotations

import dataclasses
import datetime
import decimal
import enum
import types
import uuid
from dataclasses import MISSING
from typing import Annotated, Any, Callable, Generic, Mapping, TypeVar, cast, get_args, get_origin, get_type_hints
from typing_extensions import dataclass_transform

T = TypeVar("T")
S = TypeVar("S")
U = TypeVar("U")


@dataclasses.dataclass(frozen=True, slots=True)
class Vector:
    dimensions: int

    def __post_init__(self) -> None:
        if self.dimensions <= 0:
            raise ValueError("vector dimensions must be positive")


def vector(dimensions: int) -> Vector:
    return Vector(dimensions)


def field(*, default: Any = MISSING, default_factory: Callable[[], Any] | Any = MISSING,
          description: str | None = None) -> Any:
    metadata = {"tesela_description": description} if description else {}
    arguments: dict[str, Any] = {"metadata": metadata}
    if default is not MISSING:
        arguments["default"] = default
    if default_factory is not MISSING:
        arguments["default_factory"] = default_factory
    return dataclasses.field(**arguments)


def _snake(name: str) -> str:
    value = "".join(("_" if index and char.isupper() else "") + char.lower()
                    for index, char in enumerate(name))
    if not value or not value[0].islower() or not all(c.islower() or c.isdigit() or c == "_" for c in value):
        raise ValueError(f"invalid semantic name {value!r}")
    return value


def _type_definition(annotation: Any) -> tuple[Any, bool, dict[str, Any]]:
    nullable = False
    origin = get_origin(annotation)
    if origin in (types.UnionType, __import__("typing").Union):
        args = get_args(annotation)
        if type(None) in args:
            nullable = True
            annotation = next(item for item in args if item is not type(None))
            origin = get_origin(annotation)
    if origin is Annotated:
        annotation, *metadata = get_args(annotation)
        marker = next((item for item in metadata if isinstance(item, Vector)), None)
        if marker:
            return annotation, nullable, {"data_type": {"vector": marker.dimensions}}
        origin = get_origin(annotation)
    if isinstance(annotation, type) and issubclass(annotation, enum.Enum):
        return annotation, nullable, {"data_type": "enum", "allowed_values": [item.value for item in annotation]}
    mapping = {str: "string", int: "integer", float: "float", bool: "boolean", dict: "json",
               uuid.UUID: "uuid", datetime.date: "date", datetime.datetime: "timestamptz",
               decimal.Decimal: "decimal"}
    if origin is list:
        return annotation, nullable, {"data_type": "array"}
    if annotation not in mapping:
        raise TypeError(f"unsupported property annotation {annotation!r}")
    return annotation, nullable, {"data_type": mapping[annotation]}


@dataclass_transform(field_specifiers=(field,), frozen_default=True)
def object(*, key: str, name: str | None = None, traits: tuple[type, ...] = ()) -> Callable[[type[T]], type[T]]:
    def decorate(cls: type[T]) -> type[T]:
        value_type = dataclasses.dataclass(frozen=True)(cls)
        hints = get_type_hints(value_type, include_extras=True)
        if key not in hints:
            raise ValueError(f"key {key!r} is not annotated")
        properties = []
        fields = {item.name: item for item in dataclasses.fields(cast(Any, value_type))}
        for field_name, annotation in hints.items():
            _, nullable, definition = _type_definition(annotation)
            description = fields[field_name].metadata.get("tesela_description")
            properties.append({"api_name": field_name, **definition,
                               **({"nullable": True} if nullable else {}),
                               **({"description": description} if description else {})})
        definition = {"api_name": name or _snake(value_type.__name__), "primary_key": key,
                      "properties": properties,
                      "traits": [getattr(item, "__tesela_definition__")["api_name"] for item in traits]}
        setattr(value_type, "__tesela_definition__", definition)
        setattr(value_type, "__tesela_hints__", hints)
        setattr(value_type, "__tesela_key__", key)
        setattr(value_type, "__tesela_traits__", traits)
        return value_type
    return decorate


def trait(cls: type[T]) -> type[T]:
    hints = get_type_hints(cls, include_extras=True)
    properties = []
    for name, annotation in hints.items():
        _, nullable, definition = _type_definition(annotation)
        properties.append({"api_name": name, **definition, **({"nullable": True} if nullable else {})})
    setattr(cls, "__tesela_definition__", {"api_name": _snake(cls.__name__), "properties": properties})
    return cls


@dataclasses.dataclass(frozen=True, slots=True)
class LinkDefinition(Generic[S, U]):
    name: str
    source: type[S]
    target: type[U]
    cardinality: str
    inverse: LinkDefinition[U, S] | None = None

    def _wire(self) -> dict[str, Any]:
        cardinality = {"one": "one_to_one", "many": "one_to_many",
                       "many_to_many": "many_to_many"}[self.cardinality]
        return {"api_name": self.name, "from": _name(self.source), "to": _name(self.target),
                "cardinality": cardinality,
                **({"inverse": self.inverse.name} if self.inverse else {})}


def link(name: str, source: type[S], target: type[U], *, cardinality: str = "many",
         inverse: LinkDefinition[U, S] | None = None) -> LinkDefinition[S, U]:
    if cardinality not in {"one", "many", "many_to_many"}:
        raise ValueError("cardinality must be 'one', 'many', or 'many_to_many'")
    return LinkDefinition(name, source, target, cardinality, inverse)


@dataclass_transform(field_specifiers=(field,), frozen_default=True)
def action(*, subject: type | None = None, returns: type,
           reads: tuple[type, ...] = (), writes: tuple[type, ...] = (),
           creates: tuple[type, ...] = (), external: tuple[str, ...] = (),
           pure: bool = False, idempotent: bool = False,
           concurrency: str = "serial", approval: str = "policy", risk: str = "low") -> Callable[[type[T]], type[T]]:
    if not pure and not (reads or writes or creates or external):
        raise ValueError("actions must declare effects or pure=True")
    if concurrency not in {"serial", "optimistic", "unrestricted"}:
        raise ValueError("concurrency must be 'serial', 'optimistic', or 'unrestricted'")
    if approval not in {"never", "policy", "always"}:
        raise ValueError("approval must be 'never', 'policy', or 'always'")
    if risk not in {"low", "medium", "high"}:
        raise ValueError("risk must be 'low', 'medium', or 'high'")
    def decorate(cls: type[T]) -> type[T]:
        command = dataclasses.dataclass(frozen=True)(cls)
        hints = get_type_hints(command, include_extras=True)
        properties: dict[str, Any] = {}
        required: list[str] = []
        for name, annotation in hints.items():
            _, nullable, definition = _type_definition(annotation)
            properties[name] = {"type": definition["data_type"] if isinstance(definition["data_type"], str) else "array"}
            if not nullable:
                required.append(name)
        definition = {
            "api_name": _snake(command.__name__), "subject": _name(subject) if subject else None,
            "input_schema": {"type": "object", "properties": properties, "required": required},
            "reads": [_name(item) for item in reads], "writes": [_name(item) for item in writes],
            "creates": [_name(item) for item in creates], "external_effects": list(external),
            "effects": {"reads": bool(reads), "writes": bool(writes or creates),
                        "external": bool(external), "idempotent": pure or idempotent},
            "concurrency": concurrency, "approval": approval, "risk_level": risk,
        }
        setattr(command, "__tesela_action__", definition)
        setattr(command, "__tesela_returns__", returns)
        return command
    return decorate


def _name(value: type | str | None) -> str:
    if isinstance(value, str):
        return value
    if value is None:
        raise TypeError("semantic definition is required")
    return str(getattr(value, "__tesela_definition__", {}).get("api_name") or
               getattr(value, "__tesela_action__", {}).get("api_name") or _snake(value.__name__))


def encode_model(value: Any) -> dict[str, Any]:
    return {item.name: getattr(value, item.name) for item in dataclasses.fields(value)}


def model_definition(value: type) -> Mapping[str, Any]:
    definition = getattr(value, "__tesela_definition__", None)
    if definition is None:
        raise TypeError(f"{value!r} is not a @tesela.object")
    return definition
