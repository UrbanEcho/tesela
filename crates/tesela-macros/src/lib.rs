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
//! The `Object`, `Link`, `Trait`, and `Action` derives implement the typed
//! semantic facade. Definitions contain no executable code or storage mapping.

mod action_macro;
mod helpers;
mod link_macro;
mod object_type_macro;
mod trait_macro;

use proc_macro::TokenStream;

/// Derive a typed semantic object for a struct.
///
/// Struct-level: `#[tesela(key = id, name = "...", display = "...", description = "...")]`
/// Field-level: `#[tesela(nullable, data_type = tesela::model::DataType::TimestampTz, description = "...")]`
///
/// `key` is required. A record's identity is part of its meaning, so
/// the macro will not invent one.
#[proc_macro_derive(Object, attributes(tesela))]
pub fn derive_object(input: TokenStream) -> TokenStream {
    object_type_macro::expand(input)
}

/// Derive a typed action command with explicit effects and output type.
#[proc_macro_derive(Action, attributes(tesela))]
pub fn derive_action(input: TokenStream) -> TokenStream {
    action_macro::expand(input)
}

/// Derive a typed semantic link.
///
/// Struct-level: `#[tesela(from = Source, to = Target, cardinality = many)]`.
///
/// `from` and `to` are required. The derived link type is directional and
/// carries no storage mapping: a host binds a link to a resolver by link id.
#[proc_macro_derive(Link, attributes(tesela))]
pub fn derive_link(input: TokenStream) -> TokenStream {
    link_macro::expand(input)
}

/// Derive a reusable semantic trait definition.
///
/// Struct-level: `#[tesela(name = "...", display = "...", description = "...")]`
/// Field-level: `#[tesela(nullable, data_type = ..., description = "...")]`
#[proc_macro_derive(Trait, attributes(tesela))]
pub fn derive_trait(input: TokenStream) -> TokenStream {
    trait_macro::expand(input)
}
