//! The one opaque keyset cursor.
//!
//! Pagination has exactly one mechanism. A [`Cursor`] encodes the last
//! returned record's sort values and primary key, bound by a fingerprint to the
//! scope, filter and sort it was issued for. A page is every record strictly
//! after that position in the order defined by [`compare_records`]. Because the
//! position is a value tuple rather than a row index, a walk is stable under
//! ties, under inserts or deletes before the cursor, and under deletion of the
//! cursor row itself.
//!
//! The token is not authenticated and not secret: it holds the last row's sort
//! values. Callers must therefore never build one over a property the caller
//! may not read; the runtime rejects a sort on a redacted property. Filter,
//! policy, guards and authority still apply to every row.
//!
//! [`compare_records`]: crate::order::compare_records

use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tesela_core::{ApiName, Error, FilterOp, Key, Value};

use crate::order::{Sort, SortDirection, compare_slot};
use crate::{Filter, Record};

/// Current token format version. An unknown version is rejected.
const VERSION: u8 = 1;

/// An opaque position in a query's total order.
///
/// Callers treat it as a string to hand back; only [`Cursor::encode`] makes
/// one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Cursor(String);

impl Cursor {
    /// The token text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Cursor {
    type Err = Error;

    /// Wrap caller-supplied token text. The text is validated by
    /// [`Cursor::decode`], where scope, filter and sort are known.
    fn from_str(token: &str) -> Result<Self, Error> {
        Ok(Self(token.to_owned()))
    }
}

/// The wire form inside the hex token.
#[derive(Serialize, Deserialize)]
struct Token {
    v: u8,
    f: String,
    s: Vec<Vec<Value>>,
    k: Key,
}

/// The decoded position of the last row a previous page returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeysetPosition {
    values: Vec<Option<Value>>,
    primary_key: Key,
}

/// Fingerprint of the parts of a query a cursor is bound to.
///
/// `limit` and `fields` are excluded so page size and projection may change
/// mid-walk. Policy-injected filters belong in `filter`, so a cursor cannot
/// cross principals with different row scopes. `serde_json` must keep its
/// `preserve_order` feature off: maps then serialize in sorted order and the
/// bytes are deterministic.
#[must_use]
pub fn fingerprint(scope: &str, filter: Option<&Filter>, sort: &[Sort]) -> [u8; 16] {
    let bytes = serde_json::to_vec(&serde_json::json!({
        "v": VERSION,
        "scope": scope,
        "filter": filter,
        "sort": sort,
    }))
    .expect("a filter and sort list always serialize to JSON");
    let digest = Sha256::digest(bytes);
    let mut out = [0_u8; 16];
    out.copy_from_slice(&digest[..16]);
    out
}

fn normalize(value: Option<&Value>) -> Option<Value> {
    value.filter(|value| !value.is_null()).cloned()
}

impl Cursor {
    /// Encode the position of `record`, the last row of a page.
    #[must_use]
    pub fn encode(scope: &str, filter: Option<&Filter>, sort: &[Sort], record: &Record) -> Self {
        Self::encode_row(
            scope,
            filter,
            sort,
            sort.iter().map(|item| record.values.get(&item.property)),
            &record.primary_key,
        )
    }

    /// Encode the position of a row that is not a [`Record`].
    ///
    /// `values` yields the row's value for each sort key, in order; `None`
    /// means the property is absent. Scan engines use this so the last row of
    /// a page needs no intermediate record.
    #[must_use]
    pub fn encode_row<'a>(
        scope: &str,
        filter: Option<&Filter>,
        sort: &[Sort],
        values: impl IntoIterator<Item = Option<&'a Value>>,
        primary_key: &Key,
    ) -> Self {
        let token = Token {
            v: VERSION,
            f: hex::encode(fingerprint(scope, filter, sort)),
            s: values
                .into_iter()
                .map(|value| normalize(value).into_iter().collect())
                .collect(),
            k: primary_key.clone(),
        };
        let bytes = serde_json::to_vec(&token).expect("a cursor token always serializes");
        Self(hex::encode(bytes))
    }

    /// Reject an empty token before any scope or filter is known. There is no
    /// length bound: a token is one position whose size follows the last row's
    /// sort values, and a bound below that would make later pages unreachable.
    pub fn check_token(&self) -> Result<(), Error> {
        if self.0.is_empty() {
            return Err(Error::validation(
                "cursor is empty; omit it to start, or pass the cursor exactly as returned",
            ));
        }
        Ok(())
    }

    /// Decode and validate this cursor against the query it is used with.
    ///
    /// Every failure is a distinct validation error.
    pub fn decode(
        &self,
        scope: &str,
        filter: Option<&Filter>,
        sort: &[Sort],
    ) -> Result<KeysetPosition, Error> {
        self.check_token()?;
        let bytes = hex::decode(&self.0).map_err(|error| {
            Error::validation(format!(
                "cursor is not valid hex ({error}); pass the cursor exactly as returned"
            ))
        })?;
        let token: Token = serde_json::from_slice(&bytes).map_err(|error| {
            Error::validation(format!(
                "cursor payload is not valid ({error}); pass the cursor exactly as returned"
            ))
        })?;
        if token.v != VERSION {
            return Err(Error::validation(format!(
                "cursor version {} is not supported (expected {VERSION}); restart without a cursor",
                token.v
            )));
        }
        if token.f != hex::encode(fingerprint(scope, filter, sort)) {
            return Err(Error::validation(
                "cursor belongs to a different query (scope, filter or sort changed); restart without a cursor",
            ));
        }
        if token.s.len() != sort.len() || token.s.iter().any(|slot| slot.len() > 1) {
            return Err(Error::validation(format!(
                "cursor holds {} sort slots but the query sorts by {}; restart without a cursor",
                token.s.len(),
                sort.len()
            )));
        }
        let values = token
            .s
            .into_iter()
            .map(|slot| slot.into_iter().next().filter(|value| !value.is_null()))
            .collect();
        Ok(KeysetPosition {
            values,
            primary_key: token.k,
        })
    }
}

impl KeysetPosition {
    /// The last returned row's primary key.
    #[must_use]
    pub fn primary_key(&self) -> &Key {
        &self.primary_key
    }

    /// The last returned row's value for each sort key; `None` is missing.
    #[must_use]
    pub fn slots(&self) -> &[Option<Value>] {
        &self.values
    }

    /// Order `record` relative to this position under `sort`.
    ///
    /// `Greater` means the record comes strictly after the position.
    #[must_use]
    pub fn compare(&self, record: &Record, sort: &[Sort]) -> Ordering {
        for (item, position) in sort.iter().zip(&self.values) {
            let ordering = compare_slot(
                record.values.get(&item.property),
                position.as_ref(),
                item.direction,
            );
            if !ordering.is_eq() {
                return ordering;
            }
        }
        record.primary_key.cmp(&self.primary_key)
    }

    /// Whether `record` comes strictly after this position.
    #[must_use]
    pub fn is_after(&self, record: &Record, sort: &[Sort]) -> bool {
        self.compare(record, sort) == Ordering::Greater
    }

    /// The pushdown predicate equivalent to [`KeysetPosition::is_after`], for
    /// backends that filter natively.
    ///
    /// It is an OR over `i` of (AND of equality on slots before `i`) AND
    /// "after slot `i`", plus the branch where every slot is equal and the
    /// primary key is greater. `primary_key` names the property that holds the
    /// primary key. Equality on a missing slot is `IsNull`.
    #[must_use]
    pub fn to_filter(&self, sort: &[Sort], primary_key: &ApiName) -> Filter {
        let equal = |item: &Sort, position: &Option<Value>| match position {
            Some(value) => Filter::eq(item.property.clone(), value.clone()),
            None => Filter::is_null(item.property.clone()),
        };
        let mut branches = Vec::with_capacity(sort.len() + 1);
        for (index, (item, position)) in sort.iter().zip(&self.values).enumerate() {
            let after = match (item.direction, position) {
                (SortDirection::Asc, Some(value)) => Some(Filter::scalar(
                    FilterOp::Gt,
                    item.property.clone(),
                    value.clone(),
                )),
                (SortDirection::Asc, None) => Some(Filter::is_not_null(item.property.clone())),
                (SortDirection::Desc, Some(value)) => Some(Filter::or(vec![
                    Filter::scalar(FilterOp::Lt, item.property.clone(), value.clone()),
                    Filter::is_null(item.property.clone()),
                ])),
                (SortDirection::Desc, None) => None,
            };
            let Some(after) = after else { continue };
            let mut conjunction: Vec<Filter> = sort
                .iter()
                .zip(&self.values)
                .take(index)
                .map(|(item, position)| equal(item, position))
                .collect();
            conjunction.push(after);
            branches.push(Filter::and(conjunction));
        }
        let mut tail: Vec<Filter> = sort
            .iter()
            .zip(&self.values)
            .map(|(item, position)| equal(item, position))
            .collect();
        // A `Key` is the projection of a JSON value, so it always serializes.
        let key =
            serde_json::to_value(&self.primary_key).map_or_else(|_| Value::null(), Value::new);
        tail.push(Filter::scalar(FilterOp::Gt, primary_key.clone(), key));
        branches.push(Filter::and(tail));
        Filter::or(branches)
    }
}

#[cfg(test)]
mod tests;
