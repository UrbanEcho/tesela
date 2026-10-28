//! Atomic mutation application with an undo log.

use tesela_core::ErrorCode;

use super::table::{Columns, Row, Table};
use super::*;
use crate::residual::Predicate;

/// Prior state of every row a batch touched, oldest first.
pub(super) type Undo = Vec<(Key, Option<Row>)>;

pub(super) fn check_revision(row: &Row, expected: Option<&Revision>) -> Result<(), Error> {
    if let Some(expected) = expected
        && &row.revision != expected
    {
        return Err(Error::stale_revision(format!(
            "expected revision '{expected}', found '{}'",
            row.revision
        )));
    }
    Ok(())
}

fn check_preconditions(
    table: &Table,
    row: &Row,
    preconditions: &[tesela_ir::Filter],
) -> Result<(), Error> {
    let access = Columns(table);
    for precondition in preconditions {
        if !Predicate::compile(&access, precondition)?.matches(&access, &row) {
            return Err(Error::new(
                ErrorCode::PreconditionFailed,
                "mutation precondition was not satisfied",
            ));
        }
    }
    Ok(())
}

fn missing_subject() -> Error {
    Error::new(
        ErrorCode::PreconditionFailed,
        "mutation subject does not exist",
    )
}

/// Undo every logged change, newest first.
pub(super) fn roll_back(table: &mut Table, undo: Undo) {
    for (key, row) in undo.into_iter().rev() {
        table.replace(&key, row);
    }
}

/// Apply one mutation in place, logging the prior state of the touched row.
pub(super) fn apply(
    table: &mut Table,
    primary_key: &ApiName,
    mutation: &Mutation,
    next_revision: impl Fn() -> Revision,
    undo: &mut Undo,
) -> Result<i64, Error> {
    match mutation {
        Mutation::Create { values } => {
            let key = values.get(primary_key).map(Key::from).ok_or_else(|| {
                Error::bad_request(format!("memory batch create requires '{primary_key}'"))
            })?;
            if table.rows.contains_key(&key) {
                return Err(Error::conflict(format!("record '{key}' already exists")));
            }
            let row = table.row(key.clone(), values, next_revision());
            table.replace(&key, Some(row));
            undo.push((key, None));
            Ok(1)
        }
        Mutation::Update {
            primary_key,
            values,
            expected_revision,
            preconditions,
        } => {
            let row = table
                .rows
                .get(primary_key)
                .ok_or_else(|| Error::not_found("record", primary_key))?;
            check_revision(row, expected_revision.as_ref())?;
            check_preconditions(table, row, preconditions)?;
            let old = table.patch(primary_key, values, next_revision(), true);
            undo.push((primary_key.clone(), old));
            Ok(1)
        }
        Mutation::Delete {
            primary_key,
            expected_revision,
            preconditions,
        } => {
            if let Some(row) = table.rows.get(primary_key) {
                check_revision(row, expected_revision.as_ref())?;
                check_preconditions(table, row, preconditions)?;
            } else if !preconditions.is_empty() {
                return Err(missing_subject());
            }
            let old = table.replace(primary_key, None);
            let removed = i64::from(old.is_some());
            if old.is_some() {
                undo.push((primary_key.clone(), old));
            }
            Ok(removed)
        }
        Mutation::Upsert {
            values,
            expected_revision,
            preconditions,
        } => {
            let key = values.get(primary_key).map(Key::from).ok_or_else(|| {
                Error::bad_request(format!("memory batch upsert requires '{primary_key}'"))
            })?;
            if let Some(row) = table.rows.get(&key) {
                check_revision(row, expected_revision.as_ref())?;
                check_preconditions(table, row, preconditions)?;
            } else if expected_revision.is_some() {
                return Err(Error::stale_revision(
                    "record does not exist at the expected revision",
                ));
            } else if !preconditions.is_empty() {
                return Err(missing_subject());
            }
            let row = table.row(key.clone(), values, next_revision());
            let old = table.replace(&key, Some(row));
            undo.push((key, old));
            Ok(1)
        }
        Mutation::Batch { .. } => Err(Error::bad_request(
            "nested atomic batches are not supported",
        )),
    }
}
