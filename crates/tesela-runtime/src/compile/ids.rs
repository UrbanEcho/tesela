//! Dense numeric handles assigned by the compiler.
//!
//! The IR names things; execution numbers them. Every hot path in the runtime —
//! binding a backend, resolving a filter field, reading a group key, stamping a
//! receipt — uses these ids, and [`ApiName`](tesela_core::ApiName) lookups
//! happen only where a name arrives from outside (a request, a tool call, a
//! spec being compiled).
//!
//! Ids are dense (`0..len`), so a `Vec` indexed by one is a valid lookup table,
//! and they are only meaningful next to the [`CompiledOntology`] that minted
//! them.
//!
//! [`CompiledOntology`]: super::CompiledOntology

/// Generates a `u32` newtype handle with a dense index.
macro_rules! define_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(transparent)]
        pub struct $name(u32);

        impl $name {
            /// Wrap a dense index.
            #[must_use]
            pub const fn new(index: u32) -> Self {
                Self(index)
            }

            /// The dense index, suitable for a `Vec` lookup.
            #[must_use]
            pub const fn index(self) -> usize {
                self.0 as usize
            }
        }

        impl From<u32> for $name {
            fn from(index: u32) -> Self {
                Self(index)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

define_id!(
    /// Identifies an object type within one compiled ontology.
    ObjectTypeId
);
define_id!(
    /// Identifies a property within its object type.
    ///
    /// Property ids are *type-local*: property 3 of `customer` and property 3 of
    /// `order` are unrelated, so a `FieldRef` always carries the object type it
    /// was resolved against. That keeps per-type property tables dense instead
    /// of forcing one global table with holes in it.
    PropertyId
);
define_id!(
    /// Identifies a link type within one compiled ontology.
    LinkId
);
define_id!(
    /// Identifies an action within one compiled ontology.
    ActionId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_dense_indices() {
        assert_eq!(ObjectTypeId::new(2).index(), 2);
        assert_eq!(PropertyId::from(5).index(), 5);
        assert_eq!(LinkId::new(0).to_string(), "0");
    }

    #[test]
    fn ids_order_numerically() {
        assert!(ActionId::new(1) < ActionId::new(2));
        let mut ids = [ObjectTypeId::new(3), ObjectTypeId::new(1)];
        ids.sort();
        assert_eq!(ids[0], ObjectTypeId::new(1));
    }
}
