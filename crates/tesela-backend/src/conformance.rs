//! Reusable behavioral checks for backend implementations.

use std::collections::BTreeMap;

use tesela_core::{ApiName, Error, Key, Value};

use crate::{Backend, Mutation, Query, Sort, SortDirection};

/// Isolated data used by the backend conformance kit.
#[derive(Debug, Clone)]
pub struct ConformanceFixture {
    /// Object type dedicated to this run.
    pub object: ApiName,
    /// Primary-key property.
    pub primary_key: ApiName,
    /// Two distinct records with their explicit keys.
    pub records: [(Key, BTreeMap<ApiName, Value>); 2],
}

/// Exercise advertised operations, read consistency, revisions, conflicts,
/// atomic primitives, and cleanup against an isolated backend instance.
pub fn verify(backend: &dyn Backend, fixture: &ConformanceFixture) -> Result<(), Error> {
    let capabilities = backend.capabilities();
    if capabilities.create {
        for (key, values) in &fixture.records {
            backend.create(&fixture.object, key.clone(), values.clone())?;
        }
        let (key, values) = &fixture.records[0];
        if backend
            .create(&fixture.object, key.clone(), values.clone())
            .is_ok()
        {
            return Err(Error::internal("duplicate create did not conflict"));
        }
    }
    if capabilities.get {
        for (key, _) in &fixture.records {
            let record = backend
                .get(&fixture.object, key)?
                .ok_or_else(|| Error::internal("created record was not readable"))?;
            if &record.primary_key != key {
                return Err(Error::internal("get returned the wrong primary key"));
            }
        }
    }
    if capabilities.search {
        let page = backend.search(
            &fixture.object,
            &Query {
                limit: Some(1),
                ..Query::default()
            },
        )?;
        if page.records.len() != 1 || page.next_cursor.is_none() {
            return Err(Error::internal(
                "search pagination contract was not honored",
            ));
        }
        let next = backend.search(
            &fixture.object,
            &Query {
                limit: Some(1),
                cursor: page.next_cursor,
                ..Query::default()
            },
        )?;
        if next.records.len() != 1 {
            return Err(Error::internal("search cursor was not consistent"));
        }
        let first = &fixture.records[0];
        let filtered = backend.search(
            &fixture.object,
            &Query {
                filter: Some(tesela_ir::Filter::eq(
                    fixture.primary_key.clone(),
                    first.0.to_value(),
                )),
                ..Query::default()
            },
        )?;
        if filtered.records.len() != 1 || filtered.records[0].primary_key != first.0 {
            return Err(Error::internal(
                "search filtering was inconsistent with get",
            ));
        }
        let ordered = backend.search(
            &fixture.object,
            &Query {
                sort: vec![Sort {
                    property: fixture.primary_key.clone(),
                    direction: SortDirection::Desc,
                }],
                ..Query::default()
            },
        )?;
        if ordered.records.len() != 2
            || ordered.records[0].primary_key <= ordered.records[1].primary_key
        {
            return Err(Error::internal("search ordering was not deterministic"));
        }
    }
    if capabilities.update && capabilities.get {
        let (key, _) = &fixture.records[0];
        let original = backend
            .get(&fixture.object, key)?
            .ok_or_else(|| Error::internal("fixture disappeared"))?;
        let revision = original
            .revision
            .clone()
            .ok_or_else(|| Error::internal("mutation backend did not assign a revision"))?;
        backend.update(&fixture.object, key, BTreeMap::new(), Some(&revision))?;
        if backend
            .update(&fixture.object, key, BTreeMap::new(), Some(&revision))
            .is_ok()
        {
            return Err(Error::internal("stale revision was accepted"));
        }
    }
    if capabilities.upsert {
        let (key, values) = &fixture.records[0];
        backend.upsert(&fixture.object, key.clone(), values.clone(), None)?;
    }
    if capabilities.batch {
        let result = backend.batch(&fixture.object, &fixture.primary_key, &[])?;
        if result.rows_affected != Some(0) {
            return Err(Error::internal("empty batch was not atomic no-op"));
        }
        if capabilities.get {
            let third_value = Value::string("conformance-3");
            let third_key = Key::from(&third_value);
            let third_values = BTreeMap::from([(fixture.primary_key.clone(), third_value)]);
            let duplicate = &fixture.records[0];
            let failed = backend.batch(
                &fixture.object,
                &fixture.primary_key,
                &[
                    Mutation::Create {
                        values: third_values,
                    },
                    Mutation::Create {
                        values: duplicate.1.clone(),
                    },
                ],
            );
            if failed.is_ok() || backend.get(&fixture.object, &third_key)?.is_some() {
                return Err(Error::internal("failed batch was not atomic"));
            }
        }
    }
    if capabilities.get {
        std::thread::scope(|scope| {
            let handles: Vec<_> = fixture
                .records
                .iter()
                .map(|(key, _)| scope.spawn(move || backend.get(&fixture.object, key)))
                .collect();
            for handle in handles {
                handle
                    .join()
                    .map_err(|_| Error::internal("concurrent backend read panicked"))??;
            }
            Ok::<(), Error>(())
        })?;
    }
    if capabilities.delete {
        for (key, _) in &fixture.records {
            backend.delete(&fixture.object, key, None)?;
        }
    }
    backend.close()
}

/// Construct a compact string-key fixture for backend tests.
#[must_use]
pub fn string_fixture(namespace: &str) -> ConformanceFixture {
    let object = ApiName::new_unchecked(namespace);
    let primary_key = ApiName::new_unchecked("id");
    let record = |id: &str| {
        let value = Value::string(id);
        (
            Key::from(&value),
            BTreeMap::from([(primary_key.clone(), value)]),
        )
    };
    let records = [record("conformance-1"), record("conformance-2")];
    ConformanceFixture {
        object,
        primary_key,
        records,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryBackend;

    #[test]
    fn memory_backend_passes_the_kit() {
        verify(
            MemoryBackend::new().as_ref(),
            &string_fixture("conformance_customer"),
        )
        .expect("conformant backend");
    }
}
