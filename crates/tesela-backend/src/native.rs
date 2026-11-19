//! Keyset paging for backends that filter and sort natively.

use tesela_core::{ApiName, Error};
use tesela_ir::order::compare_records;
use tesela_ir::{Cursor, Filter, Page, Record};

use crate::Query;
use crate::residual::page_size;

/// Page a natively filtering backend by keyset.
///
/// The cursor is decoded against `scope`, then folded into the pushed-down
/// filter as the predicate "strictly after the position". `fetch` receives a
/// query with no cursor, `limit = page + 1` and that combined filter, and must
/// honour filter, sort (with the primary-key tiebreak) and limit. Its rows are
/// verified to be strictly increasing and after the position, so a backend that
/// ignores the contract fails loudly instead of corrupting a walk. The page is
/// then truncated and its cursor encodes the last returned row.
///
/// `primary_key` names the property that holds a record's primary key.
pub fn native_page(
    scope: &str,
    primary_key: &ApiName,
    query: &Query,
    fetch: impl FnOnce(&Query) -> Result<Vec<Record>, Error>,
) -> Result<Page, Error> {
    let page = page_size(query)?;
    let position = query
        .cursor
        .as_ref()
        .map(|cursor| cursor.decode(scope, query.filter.as_ref(), &query.sort))
        .transpose()?;
    let filter = match &position {
        Some(position) => {
            let after = position.to_filter(&query.sort, primary_key);
            Some(match query.filter.clone() {
                Some(user) => Filter::and(vec![user, after]),
                None => after,
            })
        }
        None => query.filter.clone(),
    };
    let pushdown = Query {
        filter,
        sort: query.sort.clone(),
        limit: Some(i32::try_from(page.saturating_add(1)).unwrap_or(i32::MAX)),
        cursor: None,
        fields: None,
    };
    let mut records = fetch(&pushdown)?;
    let ordered = records
        .windows(2)
        .all(|pair| compare_records(&pair[0], &pair[1], &query.sort).is_lt())
        && position.as_ref().is_none_or(|position| {
            records
                .first()
                .is_none_or(|first| position.is_after(first, &query.sort))
        });
    if !ordered || records.len() > page.saturating_add(1) {
        return Err(Error::adapter(
            "native backend returned rows out of keyset order",
        ));
    }
    let has_more = records.len() > page;
    records.truncate(page);
    let next_cursor = records
        .last()
        .filter(|_| has_more)
        .map(|last| Cursor::encode(scope, query.filter.as_ref(), &query.sort, last));
    Ok(Page {
        records,
        next_cursor,
    })
}
