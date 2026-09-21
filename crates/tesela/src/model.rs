//! What the data is: ontology declarations, records and scalar values.
//!
//! Everything here is *semantic*. An [`struct@ObjectType`] says an entity has a
//! primary key and these properties; it does not say where the rows live. A
//! [`struct@LinkType`] says one type relates to another; it does not say through
//! which column. That separation is what lets one [`Spec`] run against any
//! backend.
//!
//! The derive macros in this module build these declarations from Rust types;
//! they share their names with the declarations they produce.

pub use tesela_core::{
    ApiName, ApiNameSource, DataType, Key, LinkCardinality, RiskLevel, Value, Version,
};
pub use tesela_ir::{
    ActionEffects, ActionResult, ActionType, AggregateResult, Concurrency, EntityKind, LinkType,
    MutationResult, NamedSet, ObjectType, ObjectTypeDefinition, Page, Property, Record, Revision,
    SemanticFingerprint, Spec, SpecItem, StaticObjectType, StaticProperty, Trait, duplicate_names,
};

#[cfg(feature = "macros")]
pub use tesela_macros::{LinkType, ObjectType, TraitDef, action};
