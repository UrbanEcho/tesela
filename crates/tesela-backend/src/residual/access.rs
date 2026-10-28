//! Field access for rows scanned by the residual engine.

use std::marker::PhantomData;

use tesela_core::{ApiName, Key, Value};
use tesela_ir::Record;

/// Read access to the fields of one scanned row.
///
/// The residual engine resolves every field a query names to a
/// [`RowAccess::Slot`] once, before the scan, then reads values through the
/// slot per row. Backends with a dense row layout resolve a name to a column
/// index; map-shaped records use the name itself.
pub trait RowAccess {
    /// Row handed to the engine. Borrowed rows are returned as-is, so a page
    /// can be built without cloning anything but the retained window.
    type Row;
    /// Pre-resolved handle for one field name.
    type Slot: Clone;

    /// Resolve a field name once per query.
    fn slot(&self, name: &ApiName) -> Self::Slot;

    /// Read one field. `None` means the field is absent from the row.
    fn value<'r>(&self, row: &'r Self::Row, slot: &Self::Slot) -> Option<&'r Value>;

    /// Primary key of the row, used as the final sort tie breaker.
    fn key<'r>(&self, row: &'r Self::Row) -> &'r Key;
}

/// Access to borrowed [`Record`]s.
#[derive(Default)]
pub(crate) struct Borrowed<'a>(pub(crate) PhantomData<&'a Record>);

impl<'a> RowAccess for Borrowed<'a> {
    type Row = &'a Record;
    type Slot = ApiName;

    fn slot(&self, name: &ApiName) -> ApiName {
        name.clone()
    }

    fn value<'r>(&self, row: &'r &'a Record, slot: &ApiName) -> Option<&'r Value> {
        row.values.get(slot)
    }

    fn key<'r>(&self, row: &'r &'a Record) -> &'r Key {
        &row.primary_key
    }
}

/// Access to owned [`Record`]s.
pub(crate) struct Owned;

impl RowAccess for Owned {
    type Row = Record;
    type Slot = ApiName;

    fn slot(&self, name: &ApiName) -> ApiName {
        name.clone()
    }

    fn value<'r>(&self, row: &'r Record, slot: &ApiName) -> Option<&'r Value> {
        row.values.get(slot)
    }

    fn key<'r>(&self, row: &'r Record) -> &'r Key {
        &row.primary_key
    }
}
