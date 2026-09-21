use super::*;

/// One operation inside a typed atomic batch.
pub enum BatchItem<T: Object> {
    /// Create one record.
    Create(Create<T>),
    /// Update one record.
    Update(Update<T>),
    /// Delete one record.
    Delete(Delete<T>),
    /// Atomically insert or replace one record.
    Upsert(Upsert<T>),
}
impl<T: Object> From<Create<T>> for BatchItem<T> {
    fn from(value: Create<T>) -> Self {
        Self::Create(value)
    }
}
impl<T: Object> From<Update<T>> for BatchItem<T> {
    fn from(value: Update<T>) -> Self {
        Self::Update(value)
    }
}
impl<T: Object> From<Delete<T>> for BatchItem<T> {
    fn from(value: Delete<T>) -> Self {
        Self::Delete(value)
    }
}
impl<T: Object> From<Upsert<T>> for BatchItem<T> {
    fn from(value: Upsert<T>) -> Self {
        Self::Upsert(value)
    }
}

/// Atomic single-object-domain batch plan.
pub struct Batch<T: Object> {
    items: Vec<BatchItem<T>>,
}
/// Build an atomic typed batch.
#[must_use]
pub fn batch<T: Object>(items: impl IntoIterator<Item = BatchItem<T>>) -> Batch<T> {
    Batch {
        items: items.into_iter().collect(),
    }
}
impl<T: Object> sealed::Sealed for Batch<T> {}
impl<T: Object> Plan for Batch<T> {
    type Output = BatchResult;
    fn execute(
        self,
        runtime: &raw::Runtime,
        context: &crate::Context,
    ) -> Result<Execution<Self::Output>, Error> {
        let mut items = Vec::with_capacity(self.items.len());
        for item in self.items {
            items.push(match item {
                BatchItem::Create(plan) => Mutation::Create {
                    values: plan.value.encode()?,
                },
                BatchItem::Update(plan) => {
                    if let Some(error) = plan.error {
                        return Err(error);
                    }
                    Mutation::Update {
                        primary_key: encode_key(&plan.key)?,
                        values: plan.values,
                        expected_revision: plan.revision,
                        preconditions: plan.preconditions,
                    }
                }
                BatchItem::Delete(plan) => Mutation::Delete {
                    primary_key: encode_key(&plan.key)?,
                    expected_revision: plan.revision,
                    preconditions: plan.preconditions,
                },
                BatchItem::Upsert(plan) => Mutation::Upsert {
                    values: plan.value.encode()?,
                    expected_revision: plan.revision,
                    preconditions: plan.preconditions,
                },
            });
        }
        let execution = runtime.mutate(context, &T::api_name(), Mutation::Batch { items })?;
        Ok(Execution {
            value: BatchResult {
                rows_affected: execution.output.rows_affected,
            },
            receipt: execution.receipt,
            revision: None,
        })
    }
}
