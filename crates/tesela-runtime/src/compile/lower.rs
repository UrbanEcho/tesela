use super::*;

/// Check one property declaration.
pub(super) fn check_property(
    owner: &ApiName,
    owner_kind: &str,
    property: &Property,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let name = &property.api_name;
    let mut report = |message: String| {
        diagnostics.push(
            Diagnostic::error(DiagnosticCode::InvalidProperty, message).with_api_name(name.clone()),
        );
    };

    if property.data_type == DataType::Enum && property.allowed_values.is_none() {
        report(format!(
            "{owner_kind} '{owner}' declares enum property '{name}' without allowed_values, so it would accept anything"
        ));
    }
    if let DataType::Vector(dimension) = property.data_type
        && dimension == 0
    {
        report(format!(
            "{owner_kind} '{owner}' declares vector property '{name}' with dimension 0"
        ));
    }
    if let Some(allowed) = &property.allowed_values {
        if allowed.is_empty() {
            report(format!(
                "{owner_kind} '{owner}' declares property '{name}' with an empty allowed_values, which accepts nothing"
            ));
        }
        if property.is_required() && allowed.iter().any(Value::is_null) {
            report(format!(
                "{owner_kind} '{owner}' declares required property '{name}' whose allowed_values include null"
            ));
        }
    }
    if let Some(default) = &property.default {
        if default.is_null() && property.is_required() {
            report(format!(
                "{owner_kind} '{owner}' declares required property '{name}' defaulting to null"
            ));
        } else if !property.allows(default) {
            report(format!(
                "{owner_kind} '{owner}' declares property '{name}' defaulting to {default}, which its allowed_values do not include"
            ));
        }
    }

    diagnostics
}

/// Resolve an object type's primary key, reporting it when it does not resolve.
pub(super) fn primary_key_id(
    name: &ApiName,
    properties: &[CompiledProperty],
    object_type: &ObjectType,
    diagnostics: &mut Diagnostics,
) -> PropertyId {
    let found = properties
        .iter()
        .position(|property| property.api_name == *name);

    let Some(index) = found else {
        diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::BrokenReference,
                format!(
                    "object type '{}' has primary key '{name}', which is not one of its properties",
                    object_type.api_name
                ),
            )
            .with_api_name(object_type.api_name.clone()),
        );
        return PropertyId::new(0);
    };

    let property = &properties[index];
    if property.nullable {
        diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::InvalidObjectType,
                format!(
                    "object type '{}' has nullable primary key '{name}'; a record's identity cannot be absent",
                    object_type.api_name
                ),
            )
            .with_api_name(object_type.api_name.clone()),
        );
    }
    if matches!(
        property.data_type,
        DataType::Array | DataType::Geometry | DataType::Json | DataType::Vector(_)
    ) {
        diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::InvalidObjectType,
                format!(
                    "object type '{}' has primary key '{name}' of type {:?}; an identity must be a scalar",
                    object_type.api_name, property.data_type
                ),
            )
            .with_api_name(object_type.api_name.clone()),
        );
    }

    PropertyId::new(index as u32)
}

/// Lower a list of IR properties into compiled form.
pub(super) fn compile_properties(
    properties: &[Property],
    origin: &PropertyOrigin,
) -> Vec<CompiledProperty> {
    properties
        .iter()
        .enumerate()
        .map(|(index, property)| {
            compile_property(PropertyId::new(index as u32), property, origin.clone())
        })
        .collect()
}

/// Lower one IR property into compiled form.
/// Re-home a trait's compiled property on an object type.
///
/// A trait's properties are compiled once, against the trait, and then adopted
/// by every object type that declares it. Adoption re-stamps the id — property
/// ids are type-local — and records the origin, so a collision between a trait
/// property and a declared one can name the trait it came from.
pub(super) fn adopt_property(
    property: &CompiledProperty,
    id: PropertyId,
    origin: PropertyOrigin,
) -> CompiledProperty {
    CompiledProperty {
        id,
        api_name: property.api_name.clone(),
        display: property.display.clone(),
        description: property.description.clone(),
        data_type: property.data_type,
        nullable: property.nullable,
        default: property.default.clone(),
        allowed_values: property.allowed_values.clone(),
        origin,
        tags: property.tags.clone(),
    }
}

pub(super) fn compile_property(
    id: PropertyId,
    property: &Property,
    origin: PropertyOrigin,
) -> CompiledProperty {
    CompiledProperty {
        id,
        api_name: property.api_name.clone(),
        display: property.display.clone(),
        description: property.description.clone(),
        data_type: property.data_type,
        nullable: property.nullable == Some(true),
        default: property.default.clone(),
        allowed_values: property
            .allowed_values
            .as_ref()
            .map(|values| Arc::from(values.as_slice())),
        origin,
        tags: property.tags.clone(),
    }
}
