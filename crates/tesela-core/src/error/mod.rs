//! Typed, machine-readable errors for Tesela operations.

mod kinds;
mod model;
mod report;

pub use kinds::*;
pub use model::*;
pub use report::ModelReport;

#[cfg(test)]
mod tests;
