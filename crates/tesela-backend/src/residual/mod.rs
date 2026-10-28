//! Shared allocation-aware residual query execution.
//!
//! The engine is generic over [`RowAccess`], so one filter, ordering and
//! pagination implementation serves borrowed records, owned records, dense
//! backend rows and lazily encoded host objects.

mod access;
mod filter;
mod rank;

use tesela_core::Error;
use tesela_ir::{Page, Record};

pub(crate) use access::Borrowed;
use access::Owned;
pub use access::RowAccess;
pub(crate) use filter::Predicate;
use rank::{Ranking, top_k};

use crate::Query;

/// One page of rows selected by [`execute_rows`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowPage<R> {
    /// Rows of the requested window, in result order.
    pub rows: Vec<R>,
    /// Cursor of the next window when more rows matched.
    pub next_cursor: Option<String>,
}

/// Largest result vector reserved up front.
const PREALLOCATED_ROWS: usize = 1024;

/// Execute filtering, ordering and pagination over a stream of rows.
///
/// Fields are resolved to slots once. Unsorted queries stop after `limit + 1`
/// matches and sorted queries retain only the bounded top-k set, so the scan
/// never holds more than one window of rows.
pub fn execute_rows<A: RowAccess>(
    access: &A,
    rows: impl IntoIterator<Item = Result<A::Row, Error>>,
    query: &Query,
) -> Result<RowPage<A::Row>, Error> {
    let offset = resolve_offset(query);
    let limit = query.limit.unwrap_or(1000).max(0) as usize;
    let filter = query
        .filter
        .as_ref()
        .map(|filter| Predicate::compile(access, filter))
        .transpose()?;
    let admit = |row: &A::Row| {
        filter
            .as_ref()
            .is_none_or(|filter| filter.matches(access, row))
    };
    let rows = rows.into_iter();
    if query.sort.is_empty() {
        let mut page = Vec::with_capacity(limit.saturating_add(1).min(PREALLOCATED_ROWS));
        let mut matched = 0usize;
        for row in rows {
            let row = row?;
            if !admit(&row) {
                continue;
            }
            if matched >= offset {
                page.push(row);
                if page.len() > limit {
                    break;
                }
            }
            matched = matched.saturating_add(1);
        }
        let has_more = page.len() > limit;
        page.truncate(limit);
        return Ok(RowPage {
            rows: page,
            next_cursor: has_more.then(|| (offset + limit).to_string()),
        });
    }
    let ranking = Ranking::new(access, &query.sort);
    let keep = offset.saturating_add(limit).saturating_add(1);
    let selected = top_k(&ranking, rows, keep, admit)?;
    let has_more = selected.len() > offset.saturating_add(limit);
    Ok(RowPage {
        rows: selected.into_iter().skip(offset).take(limit).collect(),
        next_cursor: has_more.then(|| (offset + limit).to_string()),
    })
}

fn into_page(page: RowPage<Record>) -> Page {
    Page {
        records: page.rows,
        next_cursor: page.next_cursor,
    }
}

/// Execute filtering, ordering and pagination over borrowed records.
///
/// Backends with only a scan primitive can use this directly. Filters are
/// compiled once, unsorted queries stop after `limit + 1` matches, and sorted
/// queries retain only the bounded top-k set before cloning returned rows.
#[inline]
pub fn execute_residual<'a>(
    records: impl IntoIterator<Item = &'a Record>,
    query: &Query,
) -> Result<Page, Error> {
    let access = Borrowed::default();
    let page = execute_rows(&access, records.into_iter().map(Ok), query)?;
    Ok(Page {
        records: page.rows.into_iter().cloned().collect(),
        next_cursor: page.next_cursor,
    })
}

/// Execute a residual query while taking ownership of a streaming scan.
///
/// Unlike [`execute_residual`], this form never needs a full input collection:
/// unsorted scans retain only one page and sorted scans retain only top-k.
#[inline]
pub fn execute_residual_owned(
    records: impl IntoIterator<Item = Record>,
    query: &Query,
) -> Result<Page, Error> {
    execute_residual_try_owned(records.into_iter().map(Ok), query)
}

/// Execute a streaming residual query whose record conversion can fail.
#[inline]
pub fn execute_residual_try_owned(
    records: impl IntoIterator<Item = Result<Record, Error>>,
    query: &Query,
) -> Result<Page, Error> {
    execute_rows(&Owned, records, query).map(into_page)
}

fn resolve_offset(query: &Query) -> usize {
    query
        .cursor
        .as_deref()
        .and_then(|cursor| cursor.parse().ok())
        .or_else(|| query.offset.map(|offset| offset.max(0) as usize))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tesela_core::{ApiName, FilterOp, Key, Value};
    use tesela_ir::Filter;

    use super::*;

    #[test]
    fn limited_scan_clones_only_the_window_and_reports_more() {
        let records: Vec<_> = (0..5)
            .map(|id| Record::new(Key::I64(id), BTreeMap::new()))
            .collect();
        let page = execute_residual(
            &records,
            &Query {
                limit: Some(2),
                ..Query::default()
            },
        )
        .expect("residual query");
        assert_eq!(page.records.len(), 2);
        assert_eq!(page.next_cursor.as_deref(), Some("2"));
    }

    #[test]
    fn membership_and_like_compile_once_and_match() {
        let field = ApiName::from("name");
        let records = [Record::new(
            Key::I64(1),
            BTreeMap::from([(field.clone(), Value::string("tesela"))]),
        )];
        for filter in [
            Filter::is_in(field.clone(), vec![Value::string("tesela")]),
            Filter {
                op: FilterOp::Like,
                field: Some(field.clone()),
                value: Some(Value::string("tes%")),
                values: Vec::new(),
                args: Vec::new(),
            },
        ] {
            assert_eq!(
                execute_residual(
                    &records,
                    &Query {
                        filter: Some(filter),
                        ..Query::default()
                    }
                )
                .expect("residual query")
                .records
                .len(),
                1
            );
        }
    }
}
