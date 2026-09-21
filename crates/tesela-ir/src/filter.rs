//! The filter AST shared by queries and policy row filters.

use serde::{Deserialize, Serialize};
use tesela_core::{ApiName, FilterOp, Value};

/// A filter predicate over record values.
///
/// This is the *authoring* form: it names properties as strings and carries
/// values as [`Value`]s, which is what a host produces when it builds a query
/// or a policy injects a row filter. The compiler lowers it into a
/// property-id-keyed form that the execution engine can evaluate without
/// name lookups, and rejects the shapes a backend would otherwise have to
/// guess at — an operator missing its field, a logical operator with no
/// operands, an unknown property.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filter {
    /// Operator.
    pub op: FilterOp,
    /// Field name, for scalar operators.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<ApiName>,
    /// Single value, for scalar operators.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// Multiple values, for `In`, `NotIn` and `Between`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<Value>,
    /// Operands, for `And`, `Or` and `Not`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Filter>,
}

impl Filter {
    /// Create an equality filter.
    #[must_use]
    pub fn eq(field: ApiName, value: impl Into<Value>) -> Self {
        Self::scalar(FilterOp::Eq, field, value.into())
    }

    /// Create an inequality filter.
    #[must_use]
    pub fn ne(field: ApiName, value: impl Into<Value>) -> Self {
        Self::scalar(FilterOp::Ne, field, value.into())
    }

    /// Create a membership filter.
    #[must_use]
    pub fn is_in(field: ApiName, values: Vec<Value>) -> Self {
        Self {
            op: FilterOp::In,
            field: Some(field),
            value: None,
            values,
            args: Vec::new(),
        }
    }

    /// Create an `IS NULL` filter.
    #[must_use]
    pub fn is_null(field: ApiName) -> Self {
        Self {
            op: FilterOp::IsNull,
            field: Some(field),
            value: None,
            values: Vec::new(),
            args: Vec::new(),
        }
    }

    /// Create an `IS NOT NULL` filter.
    #[must_use]
    pub fn is_not_null(field: ApiName) -> Self {
        Self {
            op: FilterOp::IsNotNull,
            field: Some(field),
            value: None,
            values: Vec::new(),
            args: Vec::new(),
        }
    }

    /// Create a suffix predicate.
    #[must_use]
    pub fn ends_with(field: ApiName, value: impl Into<Value>) -> Self {
        Self::scalar(FilterOp::EndsWith, field, value.into())
    }

    /// Create an AND filter combining sub-filters.
    #[must_use]
    pub fn and(filters: Vec<Filter>) -> Self {
        Self::logical(FilterOp::And, filters)
    }

    /// Create an OR filter combining sub-filters.
    #[must_use]
    pub fn or(filters: Vec<Filter>) -> Self {
        Self::logical(FilterOp::Or, filters)
    }

    /// Create a NOT filter.
    #[must_use]
    pub fn negate(filter: Filter) -> Self {
        Self::logical(FilterOp::Not, vec![filter])
    }

    /// The field this filter names, when it is a scalar filter.
    #[must_use]
    pub fn field(&self) -> Option<&ApiName> {
        self.field.as_ref()
    }

    /// Combine two filters with AND, treating either as optional.
    #[must_use]
    pub fn all(filters: impl IntoIterator<Item = Filter>) -> Option<Filter> {
        let mut collected: Vec<Filter> = filters.into_iter().collect();
        match collected.len() {
            0 => None,
            1 => collected.pop(),
            _ => Some(Filter::and(collected)),
        }
    }

    fn scalar(op: FilterOp, field: ApiName, value: Value) -> Self {
        Self {
            op,
            field: Some(field),
            value: Some(value),
            values: Vec::new(),
            args: Vec::new(),
        }
    }

    fn logical(op: FilterOp, args: Vec<Filter>) -> Self {
        Self {
            op,
            field: None,
            value: None,
            values: Vec::new(),
            args,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eq_names_its_field_and_value() {
        let filter = Filter::eq(ApiName::from("tenant_id"), Value::string("acme"));
        assert_eq!(filter.op, FilterOp::Eq);
        assert_eq!(filter.field().map(ApiName::as_str), Some("tenant_id"));
        assert_eq!(filter.value, Some(Value::string("acme")));
    }

    #[test]
    fn logical_filters_carry_operands_not_fields() {
        let filter = Filter::and(vec![
            Filter::eq(ApiName::from("a"), Value::integer(1)),
            Filter::eq(ApiName::from("b"), Value::integer(2)),
        ]);
        assert!(filter.field.is_none());
        assert_eq!(filter.args.len(), 2);
    }

    #[test]
    fn all_collapses_single_filters_and_empty_sets() {
        assert!(Filter::all(Vec::new()).is_none());
        let single = Filter::eq(ApiName::from("a"), Value::integer(1));
        assert_eq!(Filter::all(vec![single.clone()]), Some(single.clone()));
        assert_eq!(
            Filter::all(vec![
                single,
                Filter::eq(ApiName::from("b"), Value::integer(2))
            ])
            .map(|filter| filter.args.len()),
            Some(2)
        );
    }
}
