//! Actor-specific capabilities derived from the compiled ontology.
//!
//! Catalogs are stamped with the ontology generation and fingerprint. Their
//! calls still enter through [`Runtime`](crate::runtime::Runtime), so discovery
//! never bypasses authority, policy, validation, or budgets.

pub use tesela_capability::*;
