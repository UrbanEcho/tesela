//! Query planning with resolved fields and bounded pagination.

use tesela_backend::{Query, SortDirection};
use tesela_core::{Error, Value};

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
    offset: usize,
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
        let requested = query
            .limit
            .map_or(maximum_limit, |limit| limit.max(0) as usize);
        let offset = query
            .cursor
            .as_deref()
            .and_then(|cursor| cursor.parse().ok())
            .unwrap_or_else(|| query.offset.unwrap_or_default().max(0) as usize);
        Ok(Self {
            filter,
            sort,
            limit: requested.min(maximum_limit),
            offset,
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
    /// Resolved offset or cursor position.
    #[must_use]
    pub fn offset(&self) -> usize {
        self.offset
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
    use tesela_ir::Filter;

    use super::*;

    #[test]
    fn planning_resolves_fields_and_bounds_limits() {
        let object = crate::compile::tests::sample_object_type();
        let plan = CompiledQuery::compile(
            &object,
            &Query {
                filter: Some(Filter::eq(ApiName::from("name"), Value::string("Acme"))),
                sort: vec![Sort {
                    property: ApiName::from("name"),
                    direction: SortDirection::Desc,
                }],
                limit: Some(500),
                offset: Some(3),
                cursor: None,
            },
            100,
        )
        .expect("query compiles");
        assert_eq!(plan.limit(), 100);
        assert_eq!(plan.offset(), 3);
        assert_eq!(plan.sort()[0].field().name().as_str(), "name");
        assert!(plan.matches(&[(ApiName::from("name"), Value::string("Acme"))].into()));
    }
}
