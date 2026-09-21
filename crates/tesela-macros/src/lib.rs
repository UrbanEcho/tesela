//! Procedural macros for declarative Tesela ontology definitions.
//!
//! Re-exported through the `tesela` facade crate under the `macros` feature.
//! Do not depend on this crate directly; use `tesela` instead.
//!
//! Every macro here produces a *semantic* declaration: it says what an object
//! type, link type, trait or action means. Nothing produced here names a
//! backend, a table, a column or an index — a host binds declarations to
//! backends at runtime.
//!
//! Attributes for physical mapping, executable handlers, and policy are not
//! part of the semantic declaration surface. Likewise, action bodies and
//! deployment modes are host concerns. A
//! declaration that still carries one fails to compile; the error names the
//! attribute, and this list is deliberately documentation rather than a table
//! in the error path.
//!
//! # Derive macros
//!
//! * `ObjectType` — derive a `tesela::model::ObjectType` from a struct.
//! * `LinkType` — derive a `tesela::model::LinkType` from a struct.
//! * `TraitDef` — derive a `tesela::model::Trait` from a struct.
//!
//! # Attribute macros
//!
//! * `action` — derive a `tesela::model::ActionType` from a free function, and
//!   infer an input schema from its parameters.

mod action_macro;
mod helpers;
mod link_macro;
mod object_type_macro;
mod trait_macro;

use proc_macro::TokenStream;

/// Derive `tesela_object_type()` for a struct.
///
/// Struct-level: `#[tesela(primary_key = "id", name = "...", display = "...", description = "...")]`
/// Field-level: `#[tesela(nullable, data_type = tesela::model::DataType::TimestampTz, description = "...")]`
///
/// `primary_key` is required. A record's identity is part of its meaning, so
/// the macro will not invent one.
#[proc_macro_derive(ObjectType, attributes(tesela))]
pub fn derive_object_type(input: TokenStream) -> TokenStream {
    object_type_macro::expand(input)
}

/// Marks a free function as a Tesela `ActionType` and generates a
/// companion struct with a `tesela_action_type()` associated function.
///
/// The function itself is left untouched and stays callable; the declaration
/// says what the action is, and the host binds an executor to it at runtime.
///
/// # Arguments
///
/// * `subject` — object type the action acts on. The backend bound to that
///   object type executes the action.
/// * `risk` — `"low"` (default), `"medium"` or `"high"`.
/// * `effects` — `"read_only"`, `"write"` (default) or `"external"`.
/// * `idempotent` — flag: repeating the action with the same input is safe.
/// * `concurrency` — `"serial"` (default), `"optimistic"` or `"unrestricted"`.
/// * `display` — human-readable label (default: function name).
/// * `description` — action description.
#[proc_macro_attribute]
pub fn action(args: TokenStream, input: TokenStream) -> TokenStream {
    action_macro::expand(args, input)
}

/// Derive `tesela_link_type()` for a struct.
///
/// Struct-level: `#[tesela(from = "...", to = "...", cardinality = "one_to_many", inverse = "...", name = "...", display = "...")]`
///
/// `from` and `to` are required. The derived link type is directional and
/// carries no storage mapping: a host binds a link to a resolver by link id.
#[proc_macro_derive(LinkType, attributes(tesela))]
pub fn derive_link_type(input: TokenStream) -> TokenStream {
    link_macro::expand(input)
}

/// Derive `tesela_trait()` for a struct.
///
/// Struct-level: `#[tesela(name = "...", display = "...", description = "...")]`
/// Field-level: `#[tesela(nullable, data_type = ..., description = "...")]`
#[proc_macro_derive(TraitDef, attributes(tesela))]
pub fn derive_trait_def(input: TokenStream) -> TokenStream {
    trait_macro::expand(input)
}
