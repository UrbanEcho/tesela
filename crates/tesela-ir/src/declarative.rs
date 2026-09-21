//! Declarative ontology descriptors.
//!
//! These descriptors are the authoring layer for Rust code: they are `const`-
//! friendly, `&'static str`-based mirrors of the owned IR, so a derive macro can
//! emit them without allocating at definition time. `to_ir` lowers them into the
//! owned form that is serialized, compiled and indexed.

use tesela_core::{ApiName, DataType};

use crate::{ObjectType, Property};

/// Static property descriptor used by declarative object definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticProperty {
    /// Property API name.
    pub api_name: &'static str,
    /// Optional display label.
    pub display: Option<&'static str>,
    /// Optional description.
    pub description: Option<&'static str>,
    /// Property data type.
    pub data_type: DataType,
    /// Whether the property allows null values.
    pub nullable: bool,
}

impl StaticProperty {
    /// Lower this descriptor into the owned IR property.
    #[must_use]
    pub fn to_ir(&self) -> Property {
        Property {
            api_name: ApiName::from(self.api_name),
            display: self.display.map(str::to_string),
            description: self.description.map(str::to_string),
            data_type: self.data_type,
            nullable: self.nullable.then_some(true),
            default: None,
            allowed_values: None,
            tags: Vec::new(),
        }
    }
}

/// Static object type descriptor used by declarative object definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticObjectType {
    /// Object type API name.
    pub api_name: &'static str,
    /// Optional display label.
    pub display: Option<&'static str>,
    /// Optional description.
    pub description: Option<&'static str>,
    /// Primary key property.
    pub primary_key: &'static str,
    /// Static property descriptors.
    pub properties: &'static [StaticProperty],
    /// Trait API names implemented by this object type.
    pub traits: &'static [&'static str],
    /// Tags for categorization.
    pub tags: &'static [&'static str],
}

impl StaticObjectType {
    /// Lower this descriptor into the owned IR object type.
    #[must_use]
    pub fn to_ir(&self) -> ObjectType {
        ObjectType {
            api_name: ApiName::from(self.api_name),
            display: self.display.map(str::to_string),
            description: self.description.map(str::to_string),
            primary_key: ApiName::from(self.primary_key),
            properties: self.properties.iter().map(StaticProperty::to_ir).collect(),
            traits: self
                .traits
                .iter()
                .map(|value| ApiName::from(*value))
                .collect(),
            tags: self.tags.iter().map(|value| (*value).to_string()).collect(),
            deprecated_at: None,
        }
    }
}

/// Trait implemented by Rust types that declare a Tesela object type.
pub trait ObjectTypeDefinition {
    /// The static descriptor for this object type.
    fn definition() -> StaticObjectType;

    /// Lower this object definition into the owned IR.
    #[must_use]
    fn object_type() -> ObjectType {
        Self::definition().to_ir()
    }
}
