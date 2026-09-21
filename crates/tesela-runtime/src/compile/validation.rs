use super::*;

/// Whether a value map describes a whole record or a set of changes to one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ValidationMode {
    /// Every required property must be present, directly or by default.
    Create,
    /// Only the properties named are checked.
    Patch,
}

/// Validate a value map, collecting every problem rather than stopping at one.
pub(super) fn validate_values(
    object_type: &CompiledObjectType,
    values: &BTreeMap<ApiName, Value>,
    mode: ValidationMode,
) -> Diagnostics {
    let mut diagnostics = Diagnostics::new();

    for (name, value) in values {
        let Some(property) = object_type.property(name) else {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidProperty,
                    format!(
                        "object type '{object_type}' has no property '{name}', so the value would never be read"
                    ),
                )
                .with_api_name(name.clone()),
            );
            continue;
        };
        if value.is_null() {
            if property.is_required() {
                diagnostics.push(
                    Diagnostic::error(
                        DiagnosticCode::InvalidProperty,
                        format!("property '{name}' is required and cannot be null"),
                    )
                    .with_api_name(name.clone()),
                );
            }
            continue;
        }
        if let Some(diagnostic) = check_value(property, value) {
            diagnostics.push(diagnostic);
        }
    }

    if mode == ValidationMode::Create {
        for property in &object_type.properties {
            if property.is_required()
                && property.default.is_none()
                && !values.contains_key(&property.api_name)
            {
                diagnostics.push(
                    Diagnostic::error(
                        DiagnosticCode::InvalidProperty,
                        format!(
                            "property '{}' is required by object type '{object_type}' but was not supplied",
                            property.api_name
                        ),
                    )
                    .with_api_name(property.api_name.clone()),
                );
            }
        }
        // A record whose key cannot be an identity cannot be addressed.
        let primary_key = object_type.primary_key();
        if let Some(value) = values.get(&primary_key.api_name)
            && !value.is_null()
            && tesela_core::Key::identity(value).is_err()
        {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidProperty,
                    format!(
                        "primary key '{}' must be a scalar string, integer or boolean, got {}",
                        primary_key.api_name,
                        tesela_core::Key::from(value).type_name()
                    ),
                )
                .with_api_name(primary_key.api_name.clone()),
            );
        }
    }

    diagnostics
}

/// Check one non-null value against its property's declaration.
pub(super) fn check_value(property: &CompiledProperty, value: &Value) -> Option<Diagnostic> {
    if !property.accepts_type(value) {
        return Some(
            Diagnostic::error(
                DiagnosticCode::InvalidProperty,
                format!(
                    "property '{}' expects {:?} but got {}",
                    property.api_name,
                    property.data_type,
                    describe(value)
                ),
            )
            .with_api_name(property.api_name.clone()),
        );
    }
    if !property.allows(value) {
        let allowed = property
            .allowed_values
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(Value::to_string)
            .collect::<Vec<String>>()
            .join(", ");
        return Some(
            Diagnostic::error(
                DiagnosticCode::InvalidProperty,
                format!(
                    "property '{}' does not accept {value}; allowed values are [{allowed}]",
                    property.api_name
                ),
            )
            .with_api_name(property.api_name.clone()),
        );
    }
    None
}

/// Convert collected diagnostics into a result.
pub(super) fn into_validation_result(
    object_type: &CompiledObjectType,
    diagnostics: Diagnostics,
) -> Result<(), Error> {
    if diagnostics.has_errors() {
        let count = diagnostics.errors().count();
        return Err(Error::invalid(
            format!(
                "record does not satisfy object type '{object_type}': {count} validation error{}",
                if count == 1 { "" } else { "s" }
            ),
            diagnostics.into_vec(),
        ));
    }
    Ok(())
}

/// The JSON type name of a value, for diagnostics.
pub(super) fn describe(value: &Value) -> &'static str {
    match value {
        Value(serde_json::Value::Null) => "null",
        Value(serde_json::Value::Bool(_)) => "boolean",
        Value(serde_json::Value::Number(_)) => "number",
        Value(serde_json::Value::String(_)) => "string",
        Value(serde_json::Value::Array(_)) => "array",
        Value(serde_json::Value::Object(_)) => "object",
    }
}

/// Whether a value's shape matches a declared data type.
///
/// Temporal and identifier types are carried as strings — Tesela does not
/// re-implement date parsing, and a backend that has a native type validates it
/// better than a JSON shape check ever could. What matters here is that a
/// number is not smuggled into a string property, or an array into a scalar.
pub(super) fn value_fits(value: &Value, data_type: DataType) -> bool {
    use serde_json::Value as Json;
    match data_type {
        DataType::String
        | DataType::Date
        | DataType::Timestamp
        | DataType::TimestampTz
        | DataType::Uuid
        | DataType::Enum => value.is_string(),
        DataType::Integer | DataType::BigInt => value.is_i64() || value.is_u64(),
        DataType::Float => value.is_number(),
        // Decimal strings retain arbitrary precision across JSON and Python;
        // native backends may also accept JSON numbers for compact inputs.
        DataType::Decimal => value.is_number() || value.is_string(),
        DataType::Boolean => value.is_boolean(),
        DataType::Json => true,
        DataType::Geometry => value.is_object() || value.is_array(),
        DataType::Array => value.is_array(),
        DataType::Vector(dimension) => value.as_array().is_some_and(|items| {
            items.len() == dimension as usize && items.iter().all(Json::is_number)
        }),
    }
}
