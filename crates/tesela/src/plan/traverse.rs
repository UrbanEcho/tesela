use super::*;

/// Link traversal plan.
#[derive(Debug, Clone)]
pub struct Traverse<L: Link> {
    source_key: <L::Source as Object>::Key,
    filter: Option<Filter>,
    sort: Vec<raw::Sort>,
    limit: Option<i32>,
    cursor: Option<Cursor>,
    marker: PhantomData<fn() -> L>,
}
/// Build a typed traversal.
pub fn traverse<L: Link>(key: <L::Source as Object>::Key) -> Traverse<L> {
    Traverse {
        source_key: key,
        filter: None,
        sort: Vec::new(),
        limit: None,
        cursor: None,
        marker: PhantomData,
    }
}
impl<L: Link> Traverse<L> {
    /// Add a target predicate.
    #[must_use]
    pub fn filter(mut self, predicate: Expr<L::Target>) -> Self {
        self.filter = Some(match self.filter.take() {
            Some(old) => Filter::and(vec![old, predicate.filter]),
            None => predicate.filter,
        });
        self
    }
    /// Add target ordering.
    #[must_use]
    pub fn order_by(mut self, order: Order<L::Target>) -> Self {
        self.sort.push(order.sort);
        self
    }
    /// Bound the page size.
    #[must_use]
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit.min(i32::MAX as usize) as i32);
        self
    }
    /// Resume strictly after the last row of a previous page.
    ///
    /// `cursor` is the `next_cursor` of a page of this same traversal; it is
    /// rejected when the source, link, filter or ordering differ.
    #[must_use]
    pub fn after(mut self, cursor: Cursor) -> Self {
        self.cursor = Some(cursor);
        self
    }
}
impl<L: Link> sealed::Sealed for Traverse<L> {}
impl<L: Link> Plan for Traverse<L> {
    type Output = Page<L::Target>;
    fn execute(
        self,
        runtime: &raw::Runtime,
        context: &crate::Context,
    ) -> Result<Execution<Self::Output>, Error> {
        let raw = raw::TraversalQuery {
            source_pk: encode_key(&self.source_key)?,
            filter: self.filter,
            sort: self.sort,
            limit: self.limit,
            cursor: self.cursor,
            fields: None,
        };
        let execution = runtime.traverse(context, &L::definition().api_name, raw)?;
        let items = execution
            .output
            .records
            .into_iter()
            .map(L::Target::decode)
            .collect::<Result<_, _>>()?;
        Ok(Execution {
            value: Page {
                items,
                next_cursor: execution.output.next_cursor,
            },
            receipt: execution.receipt,
            revision: None,
        })
    }
}
