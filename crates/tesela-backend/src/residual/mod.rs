//! Shared allocation-aware residual query execution.
//!
//! The engine is generic over [`RowAccess`], so one filter, ordering and
//! pagination implementation serves borrowed records, owned records, dense
//! backend rows and lazily encoded host objects.

mod access;
mod filter;
mod rank;

use tesela_core::Error;
use tesela_ir::{Cursor, Page, Record};

pub(crate) use access::Borrowed;
use access::Owned;
pub use access::RowAccess;
pub(crate) use filter::Predicate;
use rank::{Ranking, top_k};

use crate::Query;

/// One page of rows selected by [`execute_rows`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowPage<R> {
    /// Rows of the requested page, in result order.
    pub rows: Vec<R>,
    /// Cursor after the last returned row, present exactly when more rows
    /// match.
    pub next_cursor: Option<Cursor>,
}

/// Page size used when a query names no limit.
pub const DEFAULT_PAGE_SIZE: usize = 1000;

/// Largest result vector reserved up front.
const PREALLOCATED_ROWS: usize = 1024;

/// The order in which a scan yields rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanOrder {
    /// The iterator yields rows by ascending primary key, so an unsorted query
    /// can stop after one page.
    PrimaryKey,
    /// The iterator yields rows in no particular order.
    Arbitrary,
}

/// Validate and resolve a query's page size.
pub(crate) fn page_size(query: &Query) -> Result<usize, Error> {
    match query.limit {
        None => Ok(DEFAULT_PAGE_SIZE),
        Some(limit) if limit >= 1 => Ok(limit as usize),
        Some(limit) => Err(Error::validation(format!(
            "limit must be >= 1 (got {limit}); it is the page size"
        ))),
    }
}

/// Execute filtering, ordering and keyset pagination over a stream of rows.
///
/// A page is the rows strictly after the query's cursor, in the order of
/// `compare_records` (sort keys, then primary key), at most `limit` of them.
/// `scope` binds the cursor to its source. Fields are resolved to slots once.
/// An unsorted scan over [`ScanOrder::PrimaryKey`] rows stops after `limit + 1`
/// matches; every other scan keeps only a bounded top-`limit + 1` set, so the
/// scan never holds more than one page of rows however deep the cursor is.
pub fn execute_rows<A: RowAccess>(
    access: &A,
    rows: impl IntoIterator<Item = Result<A::Row, Error>>,
    query: &Query,
    scope: &str,
    order: ScanOrder,
) -> Result<RowPage<A::Row>, Error> {
    let limit = page_size(query)?;
    let position = query
        .cursor
        .as_ref()
        .map(|cursor| cursor.decode(scope, query.filter.as_ref(), &query.sort))
        .transpose()?;
    let filter = query
        .filter
        .as_ref()
        .map(|filter| Predicate::compile(access, filter))
        .transpose()?;
    let ranking = Ranking::new(access, &query.sort);
    let admit = |row: &A::Row| {
        filter
            .as_ref()
            .is_none_or(|filter| filter.matches(access, row))
            && position
                .as_ref()
                .is_none_or(|position| ranking.compare_to_position(row, position).is_gt())
    };
    let rows = rows.into_iter();
    let mut selected = if query.sort.is_empty() && order == ScanOrder::PrimaryKey {
        let mut page = Vec::with_capacity(limit.saturating_add(1).min(PREALLOCATED_ROWS));
        for row in rows {
            let row = row?;
            if admit(&row) {
                page.push(row);
                if page.len() > limit {
                    break;
                }
            }
        }
        page
    } else {
        top_k(&ranking, rows, limit.saturating_add(1), admit)?
    };
    let has_more = selected.len() > limit;
    selected.truncate(limit);
    let next_cursor = selected.last().filter(|_| has_more).map(|last| {
        Cursor::encode_row(
            scope,
            query.filter.as_ref(),
            &query.sort,
            ranking.values(last),
            access.key(last),
        )
    });
    Ok(RowPage {
        rows: selected,
        next_cursor,
    })
}

fn into_page(page: RowPage<Record>) -> Page {
    Page {
        records: page.rows,
        next_cursor: page.next_cursor,
    }
}

/// Execute filtering, ordering and keyset pagination over borrowed records.
///
/// Backends with only a scan primitive can use this directly. Filters are
/// compiled once and only the retained page is cloned. `scope` binds cursors to
/// their source and `order` states how `records` is ordered.
#[inline]
pub fn execute_residual<'a>(
    records: impl IntoIterator<Item = &'a Record>,
    query: &Query,
    scope: &str,
    order: ScanOrder,
) -> Result<Page, Error> {
    let access = Borrowed::default();
    let page = execute_rows(&access, records.into_iter().map(Ok), query, scope, order)?;
    Ok(Page {
        records: page.rows.into_iter().cloned().collect(),
        next_cursor: page.next_cursor,
    })
}

/// Execute a residual query while taking ownership of a streaming scan.
///
/// Unlike [`execute_residual`], this form never needs a full input collection.
#[inline]
pub fn execute_residual_owned(
    records: impl IntoIterator<Item = Record>,
    query: &Query,
    scope: &str,
    order: ScanOrder,
) -> Result<Page, Error> {
    execute_residual_try_owned(records.into_iter().map(Ok), query, scope, order)
}

/// Execute a streaming residual query whose record conversion can fail.
#[inline]
pub fn execute_residual_try_owned(
    records: impl IntoIterator<Item = Result<Record, Error>>,
    query: &Query,
    scope: &str,
    order: ScanOrder,
) -> Result<Page, Error> {
    execute_rows(&Owned, records, query, scope, order).map(into_page)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use tesela_core::{ApiName, FilterOp, Key, Value};
    use tesela_ir::Filter;

    use super::*;
    use crate::{Sort, SortDirection};

    fn walk(records: &[Record], query: &Query, order: ScanOrder) -> Vec<Vec<i64>> {
        let mut query = query.clone();
        let mut pages = Vec::new();
        loop {
            let page = execute_residual(records, &query, "scope", order).expect("page");
            pages.push(
                page.records
                    .iter()
                    .map(|record| match record.primary_key {
                        Key::I64(id) => id,
                        _ => unreachable!("test keys are integers"),
                    })
                    .collect(),
            );
            match page.next_cursor {
                Some(cursor) => query.cursor = Some(cursor),
                None => return pages,
            }
        }
    }

    #[test]
    fn limited_scan_clones_only_the_page_and_reports_more() {
        let records: Vec<_> = (0..5)
            .map(|id| Record::new(Key::I64(id), BTreeMap::new()))
            .collect();
        let query = Query {
            limit: Some(2),
            ..Query::default()
        };
        let page = execute_residual(&records, &query, "scope", ScanOrder::PrimaryKey)
            .expect("residual query");
        assert_eq!(page.records.len(), 2);
        assert!(page.next_cursor.is_some());
        assert_eq!(
            walk(&records, &query, ScanOrder::PrimaryKey),
            [vec![0, 1], vec![2, 3], vec![4]]
        );
    }

    #[test]
    fn arbitrary_order_unsorted_walk_visits_every_row_once_by_primary_key() {
        let records: Vec<_> = [3, 0, 4, 1, 2]
            .into_iter()
            .map(|id| Record::new(Key::I64(id), BTreeMap::new()))
            .collect();
        let query = Query {
            limit: Some(2),
            ..Query::default()
        };
        assert_eq!(
            walk(&records, &query, ScanOrder::Arbitrary),
            [vec![0, 1], vec![2, 3], vec![4]]
        );
    }

    #[test]
    fn sorted_walk_is_stable_under_ties_and_missing_values() {
        let score = ApiName::from("score");
        let records: Vec<_> = [
            (0, Some(2)),
            (1, None),
            (2, Some(2)),
            (3, Some(1)),
            (4, None),
        ]
        .into_iter()
        .map(|(id, value)| {
            let mut values = BTreeMap::new();
            if let Some(value) = value {
                values.insert(score.clone(), Value::from(value as i64));
            }
            Record::new(Key::I64(id), values)
        })
        .collect();
        let mut query = Query {
            limit: Some(2),
            sort: vec![Sort {
                property: score,
                direction: SortDirection::Desc,
            }],
            ..Query::default()
        };
        assert_eq!(
            walk(&records, &query, ScanOrder::Arbitrary),
            [vec![0, 2], vec![3, 1], vec![4]]
        );
        query.sort[0].direction = SortDirection::Asc;
        assert_eq!(
            walk(&records, &query, ScanOrder::Arbitrary),
            [vec![1, 4], vec![3, 0], vec![2]]
        );
    }

    #[test]
    fn non_positive_limit_and_foreign_cursor_are_validation_errors() {
        let records = [Record::new(Key::I64(1), BTreeMap::new())];
        for limit in [0, -1] {
            let query = Query {
                limit: Some(limit),
                ..Query::default()
            };
            assert!(execute_residual(&records, &query, "scope", ScanOrder::PrimaryKey).is_err());
        }
        let page = execute_residual(
            &records,
            &Query {
                limit: Some(1),
                ..Query::default()
            },
            "other",
            ScanOrder::PrimaryKey,
        )
        .expect("page");
        assert!(page.next_cursor.is_none());
        let cursor = Cursor::encode("other", None, &[], &records[0]);
        let query = Query {
            cursor: Some(cursor),
            ..Query::default()
        };
        assert!(execute_residual(&records, &query, "scope", ScanOrder::PrimaryKey).is_err());
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
                    },
                    "scope",
                    ScanOrder::PrimaryKey,
                )
                .expect("residual query")
                .records
                .len(),
                1
            );
        }
    }
}
