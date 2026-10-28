//! JSON Schema input contracts for capabilities, derived from the compiled ontology.

use serde_json::{Map, json};

use tesela_core::DataType;

use crate::compile::{CompiledObjectType, CompiledProperty};

use super::*;

/// The JSON type of values a data type accepts on the wire.
fn data_type_schema(data_type: DataType) -> serde_json::Value {
    match data_type {
        DataType::String
        | DataType::Decimal
        | DataType::Date
        | DataType::Timestamp
        | DataType::TimestampTz
        | DataType::Uuid => json!({"type": "string"}),
        DataType::Integer | DataType::BigInt => json!({"type": "integer"}),
        DataType::Float => json!({"type": "number"}),
        DataType::Boolean => json!({"type": "boolean"}),
        DataType::Array => json!({"type": "array"}),
        DataType::Vector(dimension) => json!({
            "type": "array",
            "items": {"type": "number"},
            "minItems": dimension,
            "maxItems": dimension,
        }),
        DataType::Json | DataType::Geometry | DataType::Enum => json!({}),
    }
}

/// Schema for one declared property, honoring nullability and closed value sets.
fn property_schema(property: &CompiledProperty) -> serde_json::Value {
    let mut schema = data_type_schema(property.data_type());
    if let Some(allowed) = property.allowed_values() {
        let mut values: Vec<serde_json::Value> =
            allowed.iter().map(|value| value.0.clone()).collect();
        if property.is_nullable() {
            values.push(serde_json::Value::Null);
        }
        schema["enum"] = json!(values);
    }
    if let Some(kind) = schema
        .get("type")
        .cloned()
        .filter(|_| property.is_nullable())
    {
        schema["type"] = json!([kind, "null"]);
    }
    schema
}

/// Schema for a record's property map. A create requires the properties that
/// have neither a default nor nullability; an update supplies only a subset.
fn values_schema(object: &CompiledObjectType, required: bool) -> serde_json::Value {
    let properties: Map<String, serde_json::Value> = object
        .properties()
        .iter()
        .map(|property| {
            (
                property.api_name().as_str().to_string(),
                property_schema(property),
            )
        })
        .collect();
    let required_names: Vec<&str> = object
        .properties()
        .iter()
        .filter(|property| required && property.is_required() && property.default_value().is_none())
        .map(|property| property.api_name().as_str())
        .collect();
    json!({
        "type": "object",
        "properties": properties,
        "required": required_names,
        "additionalProperties": false,
    })
}

/// Schema for the primary key of an object type.
fn key_schema(object: &CompiledObjectType) -> serde_json::Value {
    data_type_schema(object.primary_key().data_type())
}

/// Schema for an object search: the serialized query fields.
pub(super) fn search_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "filter": {"type": "object", "description": "Filter tree in Tesela's serialized form."},
            "sort": {"type": "array", "items": {"type": "object"}},
            "limit": {"type": "integer", "minimum": 1},
            "offset": {"type": "integer", "minimum": 0},
            "cursor": {"type": "string"},
        },
        "additionalProperties": false,
    })
}

/// Schema for a read by primary key.
pub(super) fn get_schema(object: &CompiledObjectType) -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {"key": key_schema(object)},
        "required": ["key"],
        "additionalProperties": false,
    })
}

/// Schema for a mutation body. The variant tag and `expected_revision` are
/// supplied by the invocation, not the body.
pub(super) fn mutation_schema(object: &CompiledObjectType, suffix: &str) -> serde_json::Value {
    let preconditions = json!({"type": "array", "items": {"type": "object"}});
    match suffix {
        "create" => json!({
            "type": "object",
            "properties": {"values": values_schema(object, true)},
            "required": ["values"],
            "additionalProperties": false,
        }),
        "upsert" => json!({
            "type": "object",
            "properties": {"values": values_schema(object, true), "preconditions": preconditions},
            "required": ["values"],
            "additionalProperties": false,
        }),
        "update" => json!({
            "type": "object",
            "properties": {
                "primary_key": key_schema(object),
                "values": values_schema(object, false),
                "preconditions": preconditions,
            },
            "required": ["primary_key", "values"],
            "additionalProperties": false,
        }),
        _ => json!({
            "type": "object",
            "properties": {"primary_key": key_schema(object), "preconditions": preconditions},
            "required": ["primary_key"],
            "additionalProperties": false,
        }),
    }
}

/// Schema for a traversal from one source record along a link.
pub(super) fn traverse_schema(source: Option<&CompiledObjectType>) -> serde_json::Value {
    let source_key = source.map_or_else(|| json!({}), key_schema);
    json!({
        "type": "object",
        "properties": {
            "source_pk": source_key,
            "filter": {"type": "object", "description": "Filter over the target records."},
            "sort": {"type": "array", "items": {"type": "object"}},
            "limit": {"type": "integer", "minimum": 1},
            "offset": {"type": "integer", "minimum": 0},
        },
        "required": ["source_pk"],
        "additionalProperties": false,
    })
}

/// Schema for an action's declared input, or an open object when undeclared.
pub(super) fn action_schema(declared: Option<&Value>) -> Value {
    declared
        .cloned()
        .unwrap_or_else(|| Value::new(json!({"type": "object"})))
}
