//! Dense row storage for one object type.
//!
//! Field names are interned once per table, so a row is a flat vector indexed
//! by column slot. Queries resolve every name they mention to a slot before
//! scanning, which turns the per-row field lookup into an array access.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use tesela_core::{ApiName, Key, Value};
use tesela_ir::{Record, Revision};

use crate::residual::RowAccess;

/// Field-name dictionary of one table.
#[derive(Default)]
pub(super) struct Dictionary {
    names: Vec<ApiName>,
    slots: HashMap<ApiName, u32>,
}

impl Dictionary {
    pub(super) fn slot(&self, name: &ApiName) -> Option<u32> {
        self.slots.get(name).copied()
    }

    pub(super) fn intern(&mut self, name: &ApiName) -> u32 {
        if let Some(slot) = self.slot(name) {
            return slot;
        }
        let slot = u32::try_from(self.names.len()).unwrap_or(u32::MAX);
        self.names.push(name.clone());
        self.slots.insert(name.clone(), slot);
        slot
    }
}

/// One stored record. A short `values` vector means the trailing columns are
/// absent.
#[derive(Clone)]
pub(super) struct Row {
    pub(super) key: Key,
    pub(super) revision: Revision,
    values: Vec<Option<Value>>,
}

impl Row {
    fn get(&self, slot: u32) -> Option<&Value> {
        self.values.get(slot as usize)?.as_ref()
    }

    fn set(&mut self, slot: u32, value: Value) {
        let slot = slot as usize;
        if self.values.len() <= slot {
            self.values.resize(slot + 1, None);
        }
        self.values[slot] = Some(value);
    }
}

type Index = HashMap<Key, BTreeSet<Key>>;

fn index_row(indexes: &mut HashMap<u32, Index>, row: &Row) {
    for (slot, index) in indexes {
        if let Some(value) = row.get(*slot) {
            index
                .entry(Key::from(value))
                .or_default()
                .insert(row.key.clone());
        }
    }
}

fn unindex_row(indexes: &mut HashMap<u32, Index>, row: &Row) {
    for (slot, index) in indexes {
        let Some(value) = row.get(*slot) else {
            continue;
        };
        let key = Key::from(value);
        if let Some(keys) = index.get_mut(&key) {
            keys.remove(&row.key);
            if keys.is_empty() {
                index.remove(&key);
            }
        }
    }
}

/// All rows of one object type plus the indexes built over them.
#[derive(Default)]
pub(super) struct Table {
    pub(super) dict: Dictionary,
    pub(super) rows: BTreeMap<Key, Row>,
    indexes: HashMap<u32, Index>,
}

impl Table {
    /// Build a row from field values, interning any new field names.
    pub(super) fn row(
        &mut self,
        key: Key,
        values: &BTreeMap<ApiName, Value>,
        revision: Revision,
    ) -> Row {
        let mut row = Row {
            key,
            revision,
            values: Vec::with_capacity(values.len()),
        };
        for (name, value) in values {
            row.set(self.dict.intern(name), value.clone());
        }
        row
    }

    /// Materialize a stored row as an engine record.
    pub(super) fn record(&self, row: &Row) -> Record {
        let mut record = Record::new(row.key.clone(), BTreeMap::new());
        for (name, value) in self.dict.names.iter().zip(&row.values) {
            if let Some(value) = value {
                record.values.insert(name.clone(), value.clone());
            }
        }
        record.revision = Some(row.revision.clone());
        record
    }

    /// Store or remove one row, keeping indexes consistent. Returns the row
    /// that was replaced.
    pub(super) fn replace(&mut self, key: &Key, row: Option<Row>) -> Option<Row> {
        let old = match row {
            Some(row) => self.rows.insert(key.clone(), row),
            None => self.rows.remove(key),
        };
        if !self.indexes.is_empty() {
            if let Some(old) = &old {
                unindex_row(&mut self.indexes, old);
            }
            if let Some(new) = self.rows.get(key) {
                index_row(&mut self.indexes, new);
            }
        }
        old
    }

    /// Merge `values` into an existing row in place and stamp a revision.
    /// Returns the previous row when `keep_old` is set.
    pub(super) fn patch(
        &mut self,
        key: &Key,
        values: &BTreeMap<ApiName, Value>,
        revision: Revision,
        keep_old: bool,
    ) -> Option<Row> {
        let Self {
            dict,
            rows,
            indexes,
        } = self;
        let row = rows.get_mut(key)?;
        let old = keep_old.then(|| row.clone());
        unindex_row(indexes, row);
        for (name, value) in values {
            row.set(dict.intern(name), value.clone());
        }
        row.revision = revision;
        index_row(indexes, row);
        old
    }

    /// Keys of rows whose `property` equals `source_key`, in key order. The
    /// index over the property is built on first use and then maintained by
    /// every write.
    pub(super) fn linked(&mut self, property: &ApiName, source_key: &Key) -> Vec<Key> {
        let slot = self.dict.intern(property);
        if !self.indexes.contains_key(&slot) {
            let mut index = Index::new();
            for row in self.rows.values() {
                if let Some(value) = row.get(slot) {
                    index
                        .entry(Key::from(value))
                        .or_default()
                        .insert(row.key.clone());
                }
            }
            self.indexes.insert(slot, index);
        }
        self.lookup(property, source_key).unwrap_or_default()
    }

    /// Probe an already-built index without mutating the table.
    pub(super) fn lookup(&self, property: &ApiName, source_key: &Key) -> Option<Vec<Key>> {
        let index = self.indexes.get(&self.dict.slot(property)?)?;
        Some(
            index
                .get(source_key)
                .map(|keys| keys.iter().cloned().collect())
                .unwrap_or_default(),
        )
    }
}

/// Resolved-column view of a table for the residual engine.
pub(super) struct Columns<'t>(pub(super) &'t Table);

impl<'t> RowAccess for Columns<'t> {
    type Row = &'t Row;
    type Slot = Option<u32>;

    fn slot(&self, name: &ApiName) -> Option<u32> {
        self.0.dict.slot(name)
    }

    fn value<'r>(&self, row: &'r &'t Row, slot: &Option<u32>) -> Option<&'r Value> {
        let row: &'t Row = row;
        row.get((*slot)?)
    }

    fn key<'r>(&self, row: &'r &'t Row) -> &'r Key {
        &row.key
    }
}
