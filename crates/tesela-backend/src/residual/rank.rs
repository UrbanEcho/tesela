//! Bounded top-k selection for sorted residual queries.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use tesela_core::Error;

use super::access::RowAccess;
use crate::{Sort, SortDirection};

/// Largest heap reserved up front; deeper windows grow on demand so a huge
/// requested limit never allocates more than the scan retains.
const PREALLOCATED_ROWS: usize = 1024;

/// Sort keys resolved to slots, bound to the access that reads them.
pub(crate) struct Ranking<'a, A: RowAccess> {
    access: &'a A,
    keys: Vec<(A::Slot, SortDirection)>,
}

impl<'a, A: RowAccess> Ranking<'a, A> {
    pub(crate) fn new(access: &'a A, sort: &[Sort]) -> Self {
        Self {
            access,
            keys: sort
                .iter()
                .map(|item| (access.slot(&item.property), item.direction))
                .collect(),
        }
    }

    /// Same order as `compare_records`: absent sorts first, direction flips the
    /// value order, and the primary key breaks ties.
    fn compare(&self, left: &A::Row, right: &A::Row) -> Ordering {
        for (slot, direction) in &self.keys {
            let ordering = self
                .access
                .value(left, slot)
                .cmp(&self.access.value(right, slot));
            let ordering = match direction {
                SortDirection::Asc => ordering,
                SortDirection::Desc => ordering.reverse(),
            };
            if ordering.is_ne() {
                return ordering;
            }
        }
        self.access.key(left).cmp(self.access.key(right))
    }
}

struct Ranked<'r, 'a, A: RowAccess> {
    row: A::Row,
    ranking: &'r Ranking<'a, A>,
}

impl<A: RowAccess> PartialEq for Ranked<'_, '_, A> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl<A: RowAccess> Eq for Ranked<'_, '_, A> {}
impl<A: RowAccess> PartialOrd for Ranked<'_, '_, A> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl<A: RowAccess> Ord for Ranked<'_, '_, A> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.ranking.compare(&self.row, &other.row)
    }
}

/// Keep the best `keep` rows of a fallible stream, returned best first.
///
/// A candidate that cannot beat the current worst retained row is dropped
/// after one comparison, so the scan holds at most `keep` rows.
pub(crate) fn top_k<A: RowAccess>(
    ranking: &Ranking<'_, A>,
    rows: impl Iterator<Item = Result<A::Row, Error>>,
    keep: usize,
    mut admit: impl FnMut(&A::Row) -> bool,
) -> Result<Vec<A::Row>, Error> {
    let mut heap = BinaryHeap::with_capacity(keep.min(PREALLOCATED_ROWS));
    for row in rows {
        let row = row?;
        if !admit(&row) {
            continue;
        }
        let entry = Ranked { row, ranking };
        if heap.len() < keep {
            heap.push(entry);
        } else if heap.peek().is_some_and(|worst| entry < *worst) {
            heap.pop();
            heap.push(entry);
        }
    }
    Ok(heap
        .into_sorted_vec()
        .into_iter()
        .map(|entry| entry.row)
        .collect())
}
