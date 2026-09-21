//! Typed, machine-readable errors for Tesela operations.

mod kinds;
mod model;

pub use kinds::*;
pub use model::*;

#[cfg(test)]
mod tests;
