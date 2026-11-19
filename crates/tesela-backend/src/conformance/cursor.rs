//! Cursor-walk guarantees every paging backend must honor.
//!
//! The checks run against any [`Backend`] given a hook that seeds rows, so the
//! in-memory backend, repository scans and native pushdown adapters all prove
//! the same contract: a walk over `next_cursor` reaches every row exactly once,
//! in the total order of [`compare_records`], and bad cursors and limits are
//! rejected with validation errors.

use std::collections::BTreeSet;

use tesela_core::{ApiName, Error, ErrorCode, ExecutionContext, Key, Principal, Value};
use tesela_ir::{Cursor, Filter, Page, Record};

use crate::{Backend, Query, Sort, SortDirection, compare_records};

/// Replaces every row of `object` with exactly the given records.
///
/// Called once per check with a fresh object name, so implementations may
/// ignore the name when the backend keeps a single dataset.
pub type SeedRows<'a> = dyn Fn(&ApiName, &[Record]) -> Result<(), Error> + 'a;

/// Seeding hook for the cursor-walk checks.
///
/// Seeded records carry a string primary key in `primary_key` and optional
/// integer properties `a` and `b`; the backend must accept those properties.
/// Some rows omit a property and some hold JSON `null`, and many rows share
/// sort values.
pub struct CursorFixture<'a> {
    /// Base object name; every check derives its own object from it.
    pub object: ApiName,
    /// Primary-key property.
    pub primary_key: ApiName,
    /// Hook that replaces the dataset of one object.
    pub seed: &'a SeedRows<'a>,
}

/// Number of rows in the standard dataset.
const ROWS: usize = 12;

/// Run every cursor-walk check against `backend`.
pub fn verify_cursors(backend: &dyn Backend, fixture: &CursorFixture<'_>) -> Result<(), Error> {
    if !backend.capabilities().search {
        return Ok(());
    }
    let harness = Harness {
        backend,
        fixture,
        context: ExecutionContext::trusted(Principal::new("conformance"), "conformance")?,
    };
    harness.completeness_and_uniqueness()?;
    harness.stable_under_ties()?;
    harness.unsorted_is_primary_key_order()?;
    harness.stable_under_concurrent_writes()?;
    harness.rejects_foreign_cursor()?;
    harness.rejects_malformed_cursor()?;
    harness.rejects_limit_zero_and_negative()?;
    harness.last_page_has_no_cursor()
}

struct Harness<'a> {
    backend: &'a dyn Backend,
    fixture: &'a CursorFixture<'a>,
    context: ExecutionContext,
}

fn fail(message: impl Into<String>) -> Error {
    Error::internal(message.into())
}

fn name(value: &str) -> ApiName {
    ApiName::new_unchecked(value)
}

fn sort(property: &str, direction: SortDirection) -> Sort {
    Sort {
        property: name(property),
        direction,
    }
}

fn sort_shapes() -> Vec<Vec<Sort>> {
    vec![
        vec![],
        vec![sort("a", SortDirection::Asc)],
        vec![sort("a", SortDirection::Desc)],
        vec![
            sort("a", SortDirection::Asc),
            sort("b", SortDirection::Desc),
        ],
    ]
}

fn row(id: &str, a: Option<Value>, b: Option<Value>) -> Record {
    let mut values = std::collections::BTreeMap::new();
    values.insert(name("id"), Value::string(id));
    for (property, value) in [("a", a), ("b", b)] {
        if let Some(value) = value {
            values.insert(name(property), value);
        }
    }
    Record::new(Key::string(id), values)
}

/// Twelve rows with duplicate, missing and null sort values.
fn standard_rows() -> Vec<Record> {
    (0..ROWS)
        .map(|index| {
            let number = i64::try_from(index).unwrap_or(0);
            let a = match index % 4 {
                0 => None,
                1 => Some(Value::null()),
                2 => Some(Value::from(1_i64)),
                _ => Some(Value::from(2_i64)),
            };
            let b = match index % 3 {
                0 => None,
                1 => Some(Value::from(number % 2)),
                _ => Some(Value::null()),
            };
            row(&format!("r-{index:02}"), a, b)
        })
        .collect()
}

fn keys(records: &[Record]) -> Vec<Key> {
    records
        .iter()
        .map(|record| record.primary_key.clone())
        .collect()
}

fn expected_order(records: &[Record], sort: &[Sort]) -> Vec<Key> {
    let mut ordered = records.to_vec();
    ordered.sort_by(|left, right| compare_records(left, right, sort));
    keys(&ordered)
}

fn expect_validation(result: Result<Page, Error>, needle: &str, case: &str) -> Result<(), Error> {
    match result {
        Ok(_) => Err(fail(format!("{case}: the cursor or limit was accepted"))),
        Err(error) if error.code() == ErrorCode::Validation && error.message().contains(needle) => {
            Ok(())
        }
        Err(error) => Err(fail(format!(
            "{case}: expected a validation error naming {needle:?}, got {} ({})",
            error.code().as_str(),
            error.message()
        ))),
    }
}

impl Harness<'_> {
    /// Seed `rows` into a fresh object named after `check`.
    fn fresh(&self, check: &str, rows: &[Record]) -> Result<ApiName, Error> {
        let object = name(&format!("{}_{check}", self.fixture.object));
        (self.fixture.seed)(&object, rows)?;
        Ok(object)
    }

    fn page(
        &self,
        object: &ApiName,
        sort: &[Sort],
        filter: Option<Filter>,
        limit: i32,
        cursor: Option<Cursor>,
    ) -> Result<Page, Error> {
        self.backend.search(
            &self.context,
            object,
            &Query {
                filter,
                sort: sort.to_vec(),
                limit: Some(limit),
                cursor,
                fields: None,
            },
        )
    }

    /// Follow `next_cursor` to the end and return every page.
    fn walk(
        &self,
        object: &ApiName,
        sort: &[Sort],
        limit: i32,
        mut cursor: Option<Cursor>,
    ) -> Result<Vec<Vec<Record>>, Error> {
        let mut pages = Vec::new();
        loop {
            let page = self.page(object, sort, None, limit, cursor)?;
            if page.records.is_empty() {
                return Err(fail("a page without rows must not be returned"));
            }
            pages.push(page.records);
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(pages);
            }
            if pages.len() > ROWS + 2 {
                return Err(fail("the cursor walk did not terminate"));
            }
        }
    }

    fn completeness_and_uniqueness(&self) -> Result<(), Error> {
        let rows = standard_rows();
        let object = self.fresh("completeness", &rows)?;
        for sort in sort_shapes() {
            let expected = expected_order(&rows, &sort);
            for size in [1, 2, 7, ROWS + 1] {
                let limit = i32::try_from(size).unwrap_or(i32::MAX);
                let pages = self.walk(&object, &sort, limit, None)?;
                if let Some((_, full)) = pages.split_last()
                    && full.iter().any(|page| page.len() != size)
                {
                    return Err(fail(format!(
                        "a non-final page was not exactly {size} rows long"
                    )));
                }
                let walked = keys(&pages.concat());
                let unique: BTreeSet<_> = walked.iter().collect();
                if unique.len() != walked.len() {
                    return Err(fail(format!("a row repeated across pages (limit {size})")));
                }
                if walked != expected {
                    return Err(fail(format!(
                        "the walk did not match the total order (limit {size}, sort {sort:?})"
                    )));
                }
            }
        }
        Ok(())
    }

    fn stable_under_ties(&self) -> Result<(), Error> {
        let rows: Vec<_> = (0..9)
            .map(|index| row(&format!("t-{index}"), Some(Value::from(7_i64)), None))
            .collect();
        let object = self.fresh("ties", &rows)?;
        let expected = keys(&rows);
        for direction in [SortDirection::Asc, SortDirection::Desc] {
            for size in [1, 2, 4] {
                let pages = self.walk(&object, &[sort("a", direction)], size, None)?;
                if keys(&pages.concat()) != expected {
                    return Err(fail("equal sort values did not follow primary-key order"));
                }
            }
        }
        Ok(())
    }

    fn unsorted_is_primary_key_order(&self) -> Result<(), Error> {
        let rows = standard_rows();
        let object = self.fresh("unsorted", &rows)?;
        let mut expected = keys(&rows);
        expected.sort();
        for size in [1, 5] {
            if keys(&self.walk(&object, &[], size, None)?.concat()) != expected {
                return Err(fail("an unsorted query was not in primary-key order"));
            }
        }
        Ok(())
    }

    fn stable_under_concurrent_writes(&self) -> Result<(), Error> {
        let capabilities = self.backend.capabilities();
        if !capabilities.create || !capabilities.delete {
            return Ok(());
        }
        let rows = standard_rows();
        let object = self.fresh("writes", &rows)?;
        let order = [sort("a", SortDirection::Asc)];
        let first = self.page(&object, &order, None, 4, None)?;
        let cursor = first
            .next_cursor
            .clone()
            .ok_or_else(|| fail("the first page of twelve rows had no cursor"))?;
        let last = first
            .records
            .last()
            .cloned()
            .ok_or_else(|| fail("the first page was empty"))?;
        let before = row("a-before", None, None);
        let after = row("z-after", Some(Value::from(100_i64)), None);
        for record in [&before, &after] {
            self.backend.create(
                &self.context,
                &object,
                record.primary_key.clone(),
                record.values.clone(),
            )?;
        }
        self.backend
            .delete(&self.context, &object, &last.primary_key, None)?;
        let mut all = rows;
        all.push(after.clone());
        let mut expected: Vec<_> = all
            .into_iter()
            .filter(|record| compare_records(record, &last, &order).is_gt())
            .collect();
        expected.sort_by(|left, right| compare_records(left, right, &order));
        let rest = keys(&self.walk(&object, &order, 4, Some(cursor))?.concat());
        if rest != keys(&expected) {
            return Err(fail(
                "the walk after concurrent writes skipped, repeated or shifted rows",
            ));
        }
        if rest.contains(&before.primary_key) || rest.contains(&last.primary_key) {
            return Err(fail(
                "a row before the cursor or the deleted row reappeared",
            ));
        }
        if rest.iter().any(|key| keys(&first.records).contains(key)) {
            return Err(fail("a row returned on page one was returned again"));
        }
        Ok(())
    }

    fn rejects_foreign_cursor(&self) -> Result<(), Error> {
        let rows = standard_rows();
        let object = self.fresh("foreign", &rows)?;
        let other = self.fresh("foreign_other", &rows)?;
        let order = [sort("a", SortDirection::Asc)];
        let cursor = self
            .page(&object, &order, None, 2, None)?
            .next_cursor
            .ok_or_else(|| fail("no cursor to reuse"))?;
        let filter = Filter::eq(name("a"), Value::from(1_i64));
        for (case, object, sort, filter) in [
            (
                "different sort",
                &object,
                vec![sort("a", SortDirection::Desc)],
                None,
            ),
            ("different filter", &object, order.to_vec(), Some(filter)),
            ("different object", &other, order.to_vec(), None),
        ] {
            expect_validation(
                self.page(object, &sort, filter, 2, Some(cursor.clone())),
                "different query",
                case,
            )?;
        }
        Ok(())
    }

    fn rejects_malformed_cursor(&self) -> Result<(), Error> {
        let rows = standard_rows();
        let object = self.fresh("malformed", &rows)?;
        let order = [sort("a", SortDirection::Asc)];
        let valid = self
            .page(&object, &order, None, 2, None)?
            .next_cursor
            .ok_or_else(|| fail("no cursor to corrupt"))?;
        let payload: serde_json::Value = serde_json::from_slice(
            &hex::decode(valid.as_str()).map_err(|_| fail("the cursor was not hex"))?,
        )
        .map_err(|_| fail("the cursor was not JSON"))?;
        let mutated = |field: &str, value: serde_json::Value| {
            let mut payload = payload.clone();
            payload[field] = value;
            hex_cursor(&serde_json::to_vec(&payload).unwrap_or_default())
        };
        let cases = [
            ("non-hex", token("not hex at all"), "hex"),
            ("not JSON", hex_cursor(b"not json"), "payload"),
            ("unknown version", mutated("v", 99.into()), "version"),
            (
                "slot count",
                mutated("s", serde_json::json!([])),
                "sort slots",
            ),
            ("empty", token(""), "empty"),
        ];
        for (case, cursor, needle) in cases {
            expect_validation(
                self.page(&object, &order, None, 2, Some(cursor)),
                needle,
                case,
            )?;
        }
        Ok(())
    }

    fn rejects_limit_zero_and_negative(&self) -> Result<(), Error> {
        let object = self.fresh("limits", &standard_rows())?;
        for limit in [0, -1] {
            expect_validation(
                self.page(&object, &[], None, limit, None),
                "limit",
                &format!("limit {limit}"),
            )?;
        }
        Ok(())
    }

    fn last_page_has_no_cursor(&self) -> Result<(), Error> {
        let rows = standard_rows();
        let object = self.fresh("last_page", &rows)?;
        for sort in [vec![], vec![sort("a", SortDirection::Asc)]] {
            for (size, pages) in [(4, 3), (6, 2), (12, 1)] {
                let walked = self.walk(&object, &sort, size, None)?;
                if walked.len() != pages {
                    return Err(fail(format!(
                        "limit {size} over {ROWS} rows returned {} pages instead of {pages}",
                        walked.len()
                    )));
                }
            }
        }
        Ok(())
    }
}

fn hex_cursor(bytes: &[u8]) -> Cursor {
    token(&hex::encode(bytes))
}

/// Wrap caller-supplied text; wrapping never fails, decoding validates.
fn token(text: &str) -> Cursor {
    text.parse()
        .unwrap_or_else(|_| unreachable!("wrapping token text is infallible"))
}
