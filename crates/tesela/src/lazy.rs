//! Lazily encoded scan rows for repository-backed objects.
//!
//! A repository scan yields typed values. Encoding every value into a full
//! record just to test a filter wastes work on rows the query rejects, so
//! fields are encoded on first read and only returned rows are encoded whole.

use std::cell::{OnceCell, RefCell};
use std::marker::PhantomData;

use crate::error::Error;
use crate::model::{Object, encode_key};
use crate::raw::{ApiName, Key, RowAccess, Value};

/// A scanned object whose fields are encoded on demand.
pub(crate) struct LazyRow<T: Object> {
    pub(crate) object: T,
    key: OnceCell<Key>,
    fields: Box<[OnceCell<Option<Value>>]>,
}

/// Resolves queried field names to dense slots and encodes them lazily.
///
/// Every slot is resolved before the first row is built, so a row reserves
/// exactly one cell per field the query names. The first encoding failure is
/// kept so the scan can report it; a failed field reads as absent meanwhile.
pub(crate) struct LazyAccess<T: Object> {
    names: RefCell<Vec<ApiName>>,
    failure: RefCell<Option<Error>>,
    marker: PhantomData<fn() -> T>,
}

impl<T: Object> LazyAccess<T> {
    pub(crate) fn new() -> Self {
        Self {
            names: RefCell::new(Vec::new()),
            failure: RefCell::new(None),
            marker: PhantomData,
        }
    }

    /// Wrap a scanned object. Call only after the query is compiled.
    pub(crate) fn row(&self, object: T) -> LazyRow<T> {
        let width = self.names.borrow().len();
        LazyRow {
            object,
            key: OnceCell::new(),
            fields: std::iter::repeat_with(OnceCell::new).take(width).collect(),
        }
    }

    /// The first encoding error met while reading rows, if any.
    pub(crate) fn take_failure(&self) -> Option<Error> {
        self.failure.borrow_mut().take()
    }

    fn record(&self, error: Error) {
        self.failure.borrow_mut().get_or_insert(error);
    }
}

impl<T: Object> RowAccess for LazyAccess<T> {
    type Row = LazyRow<T>;
    type Slot = usize;

    fn slot(&self, name: &ApiName) -> usize {
        let mut names = self.names.borrow_mut();
        names
            .iter()
            .position(|item| item == name)
            .unwrap_or_else(|| {
                names.push(name.clone());
                names.len() - 1
            })
    }

    fn value<'r>(&self, row: &'r LazyRow<T>, slot: &usize) -> Option<&'r Value> {
        row.fields
            .get(*slot)?
            .get_or_init(|| {
                let name = self.names.borrow()[*slot].clone();
                row.object.encode_field(&name).unwrap_or_else(|error| {
                    self.record(error);
                    None
                })
            })
            .as_ref()
    }

    fn key<'r>(&self, row: &'r LazyRow<T>) -> &'r Key {
        row.key.get_or_init(|| {
            row.object
                .key()
                .and_then(|key| encode_key(&key))
                .unwrap_or_else(|error| {
                    self.record(error);
                    Key::Null
                })
        })
    }
}
