//! Object types, traits and properties.

use serde::{Deserialize, Serialize};
use tesela_core::{ApiName, DataType, Value};

/// A reusable bundle of properties.
///
/// A trait is *semantic* inheritance: an object type that declares a trait gets
/// the trait's properties, materialized into the object type at compile time
/// with their origin recorded so collisions can be reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trait {
    /// API name.
    pub api_name: ApiName,
    /// Display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// Description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Properties the trait contributes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<Property>,
}

/// A domain entity type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectType {
    /// API name.
    pub api_name: ApiName,
    /// Display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// Description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The property that identifies a record.
    ///
    /// This must name a property the object type declares, directly or through
    /// a trait; the compiler rejects a primary key that does not resolve.
    pub primary_key: ApiName,
    /// Properties declared by this object type.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<Property>,
    /// Traits this object type implements.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub traits: Vec<ApiName>,
    /// Tags for categorization.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// When this object type was deprecated, as an ISO-8601 instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated_at: Option<String>,
}

impl ObjectType {
    /// Look a declared property up by name.
    #[must_use]
    pub fn property(&self, name: &str) -> Option<&Property> {
        self.properties
            .iter()
            .find(|property| property.api_name == name)
    }
}

/// A property on an object type or trait.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Property {
    /// API name.
    pub api_name: ApiName,
    /// Display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// Description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Data type.
    pub data_type: DataType,
    /// Whether `null` is an acceptable value.
    ///
    /// Absent means the property is required, which is the safe default: a
    /// property becomes optional only when its author says so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nullable: Option<bool>,
    /// Value applied when a create omits the property.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// The closed set of values this property accepts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_values: Option<Vec<Value>>,
    /// Tags for categorization.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl Property {
    /// Whether a record must supply this property.
    #[must_use]
    pub fn is_required(&self) -> bool {
        self.nullable != Some(true)
    }

    /// Whether this property accepts the given value.
    ///
    /// A property with no declared `allowed_values` accepts anything; one with
    /// a declared set accepts only members of it. `null` is handled by
    /// [`Property::is_required`], not here.
    #[must_use]
    pub fn allows(&self, value: &Value) -> bool {
        match &self.allowed_values {
            Some(allowed) => allowed.iter().any(|candidate| candidate == value),
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn property(name: &'static str, nullable: Option<bool>) -> Property {
        Property {
            api_name: ApiName::from(name),
            display: None,
            description: None,
            data_type: DataType::String,
            nullable,
            default: None,
            allowed_values: None,
            tags: Vec::new(),
        }
    }

    #[test]
    fn properties_are_required_by_default() {
        assert!(property("email", None).is_required());
        assert!(property("email", Some(false)).is_required());
        assert!(!property("email", Some(true)).is_required());
    }

    #[test]
    fn allowed_values_gate_membership() {
        let mut status = property("status", None);
        assert!(status.allows(&Value::string("anything")));
        status.allowed_values = Some(vec![Value::string("active")]);
        assert!(status.allows(&Value::string("active")));
        assert!(!status.allows(&Value::string("closed")));
    }
}
