//! Query planning with resolved fields and bounded pagination.

use tesela_backend::{Query, SortDirection};
use tesela_core::{Error, Value};
use tesela_ir::Cursor;

use super::{CompiledFilter, CompiledObjectType, FieldRef, compile_filter};

/// A resolved sort key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledSort {
    field: FieldRef,
    direction: SortDirection,
}

impl CompiledSort {
    /// Resolved field.
    #[must_use]
    pub fn field(&self) -> &FieldRef {
        &self.field
    }
    /// Sort direction.
    #[must_use]
    pub fn direction(&self) -> SortDirection {
        self.direction
    }
}

/// Immutable query plan resolved against one compiled object type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledQuery {
    filter: Option<CompiledFilter>,
    sort: Box<[CompiledSort]>,
    limit: usize,
    cursor: Option<Cursor>,
}

impl CompiledQuery {
    /// Resolve names, compile predicates once, and apply the runtime limit.
    pub fn compile(
        object: &CompiledObjectType,
        query: &Query,
        maximum_limit: usize,
    ) -> Result<Self, Error> {
        let filter = query
            .filter
            .as_ref()
            .map(|filter| compile_filter(object, filter))
            .transpose()?;
        let sort = query
            .sort
            .iter()
            .map(|sort| {
                object
                    .property(&sort.property)
                    .map(|property| CompiledSort {
                        field: FieldRef::new(property.id(), sort.property.clone()),
                        direction: sort.direction,
                    })
                    .ok_or_else(|| Error::unknown_property(object, &sort.property))
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_boxed_slice();
        let requested = match query.limit {
            None => maximum_limit,
            Some(limit) if limit >= 1 => limit as usize,
            Some(limit) => {
                return Err(Error::validation(format!(
                    "limit must be >= 1 (got {limit}); it is the page size"
                )));
            }
        };
        Ok(Self {
            filter,
            sort,
            limit: requested.min(maximum_limit),
            cursor: query.cursor.clone(),
        })
    }
    /// Compiled predicate.
    #[must_use]
    pub fn filter(&self) -> Option<&CompiledFilter> {
        self.filter.as_ref()
    }
    /// Resolved sort keys. Primary-key order is the implicit final key.
    #[must_use]
    pub fn sort(&self) -> &[CompiledSort] {
        &self.sort
    }
    /// Bounded page size.
    #[must_use]
    pub fn limit(&self) -> usize {
        self.limit
    }
    /// The opaque keyset cursor; the backend decodes it against its scope.
    #[must_use]
    pub fn cursor(&self) -> Option<&Cursor> {
        self.cursor.as_ref()
    }
    /// Evaluate the residual predicate without reparsing it.
    #[must_use]
    pub fn matches(
        &self,
        values: &std::collections::BTreeMap<tesela_core::ApiName, Value>,
    ) -> bool {
        self.filter
            .as_ref()
            .is_none_or(|filter| filter.matches(values))
    }
}

#[cfg(test)]
mod tests {
    use tesela_backend::{Query, Sort, SortDirection};
    use tesela_core::{ApiName, Value};
    use tesela_ir::{Filter, Record};

    use super::*;

    #[test]
    fn planning_resolves_fields_and_bounds_limits() {
        let object = crate::compile::tests::sample_object_type();
        let cursor = Cursor::encode(
            "customer",
            None,
            &[],
            &Record::new(tesela_core::Key::I64(7), Default::default()),
        );
        let plan = CompiledQuery::compile(
            &object,
            &Query {
                filter: Some(Filter::eq(ApiName::from("name"), Value::string("Acme"))),
                sort: vec![Sort {
                    property: ApiName::from("name"),
                    direction: SortDirection::Desc,
                }],
                limit: Some(500),
                cursor: Some(cursor.clone()),
                fields: None,
            },
            100,
        )
        .expect("query compiles");
        assert_eq!(plan.limit(), 100);
        assert_eq!(plan.cursor(), Some(&cursor));
        assert_eq!(plan.sort()[0].field().name().as_str(), "name");
        assert!(plan.matches(&[(ApiName::from("name"), Value::string("Acme"))].into()));
    }

    #[test]
    fn non_positive_limits_are_rejected() {
        let object = crate::compile::tests::sample_object_type();
        for limit in [0, -1] {
            let query = Query {
                limit: Some(limit),
                ..Query::default()
            };
            assert!(CompiledQuery::compile(&object, &query, 100).is_err());
        }
    }
}
