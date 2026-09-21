//! Mechanical Python bindings for the curated Tesela facade.

mod bindings;
mod conversion;

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;

create_exception!(_native, TeselaError, PyException);

#[pymodule]
#[pyo3(name = "_native")]
fn native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    bindings::register(module)
}
