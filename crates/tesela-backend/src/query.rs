//! Query, policy, and execution request types.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tesela_core::{ApiName, ExecutionContext, Key, Operation, ResourceKind, Value};
use tesela_ir::Revision;

/// A search query. Unknown keys are rejected so a stale client field (for
/// example a removed offset) fails loudly instead of being ignored.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    /// Filter tree.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub filter: Option<tesela_ir::Filter>,
    /// Sort directives.
    #[serde(default)]
    pub sort: Vec<Sort>,
    /// Page size; must be >= 1.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub limit: Option<i32>,
    /// Opaque keyset cursor: resume strictly after a previous page's last row.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cursor: Option<tesela_ir::Cursor>,
    /// Properties to return. The primary key is always kept; `None` returns
    /// every property the caller may read.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fields: Option<Vec<tesela_core::ApiName>>,
}

impl Query {
    /// Merge an additional filter with AND semantics.
    #[must_use]
    pub fn and_filter(mut self, additional: tesela_ir::Filter) -> Self {
        self.filter = Some(match self.filter.take() {
            Some(existing) => tesela_ir::Filter::and(vec![existing, additional]),
            None => additional,
        });
        self
    }
}

pub use tesela_ir::order::{Sort, SortDirection, compare_records};

/// Mutation payload accepted by the runtime convenience API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mutation {
    /// Create a record.
    Create {
        /// Field values.
        values: BTreeMap<ApiName, Value>,
    },
    /// Update a record.
    Update {
        /// Primary key.
        primary_key: Key,
        /// New values.
        values: BTreeMap<ApiName, Value>,
        /// Required current revision, when optimistic concurrency is used.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_revision: Option<Revision>,
        /// Semantic predicates that must hold on the current record.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        preconditions: Vec<tesela_ir::Filter>,
    },
    /// Delete a record.
    Delete {
        /// Primary key.
        primary_key: Key,
        /// Required current revision, when optimistic concurrency is used.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_revision: Option<Revision>,
        /// Semantic predicates that must hold on the current record.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        preconditions: Vec<tesela_ir::Filter>,
    },
    /// Insert or replace a record.
    Upsert {
        /// Field values.
        values: BTreeMap<ApiName, Value>,
        /// Required current revision for replacement; `None` permits insert.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expected_revision: Option<Revision>,
        /// Semantic predicates required when replacing an existing record.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        preconditions: Vec<tesela_ir::Filter>,
    },
    /// Apply several mutations.
    Batch {
        /// Mutations.
        items: Vec<Mutation>,
    },
}

/// Aggregate query.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AggregateQuery {
    /// Filter before aggregation.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub filter: Option<tesela_ir::Filter>,
    /// Group-by properties.
    #[serde(default)]
    pub group_by: Vec<ApiName>,
    /// Aggregations.
    #[serde(default)]
    pub aggregations: Vec<Aggregation>,
    /// Require backend pushdown.
    #[serde(default)]
    pub require_pushdown: bool,
}

/// A single aggregation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Aggregation {
    /// Aggregation function.
    pub function: AggregationFunction,
    /// Property to aggregate.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub property: Option<ApiName>,
    /// Output alias.
    pub alias: String,
}

/// Supported aggregation functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregationFunction {
    /// Count records.
    Count,
    /// Sum numeric values.
    Sum,
    /// Average numeric values.
    Avg,
    /// Minimum value.
    Min,
    /// Maximum value.
    Max,
}

impl AggregationFunction {
    /// SQL function name for pushdown-capable backends.
    #[must_use]
    pub fn as_sql_name(self) -> &'static str {
        match self {
            Self::Count => "COUNT",
            Self::Sum => "SUM",
            Self::Avg => "AVG",
            Self::Min => "MIN",
            Self::Max => "MAX",
        }
    }
}

/// Link traversal query.
///
/// A traversal starts from one record and follows one link, so it carries the
/// source key rather than a source filter: the record being traversed from is
/// addressed, not searched for. Unknown keys are rejected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraversalQuery {
    /// Primary key of the source record.
    pub source_pk: Key,
    /// Optional target-side filter.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub filter: Option<tesela_ir::Filter>,
    /// Sort directives.
    #[serde(default)]
    pub sort: Vec<Sort>,
    /// Page size; must be >= 1.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub limit: Option<i32>,
    /// Opaque keyset cursor from a previous page.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cursor: Option<tesela_ir::Cursor>,
    /// Target properties to return; the primary key is always kept.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub fields: Option<Vec<tesela_core::ApiName>>,
}

/// Request to execute an action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionRequest {
    /// Action API name.
    pub action: ApiName,
    /// Input payload.
    #[serde(default)]
    pub input: Value,
    /// Optional idempotency key or run id.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub run_id: Option<String>,
    /// Required subject revision, when the action is state-dependent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<Revision>,
    /// Semantic state predicates that must still hold before execution.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preconditions: Vec<tesela_ir::Filter>,
}

/// Borrowed policy request; constructing one performs no claims or name clones.
#[derive(Debug)]
pub struct PolicyRequest<'a> {
    /// Execution context and authenticated principal.
    pub context: &'a ExecutionContext,
    /// Operation.
    pub operation: Operation,
    /// Resource kind.
    pub resource_kind: ResourceKind,
    /// Resource name.
    pub resource: &'a ApiName,
    /// Operation input, when policy needs to constrain a mutation or action.
    pub input: Option<&'a Value>,
}

/// Policy decision.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PolicyDecision {
    /// Whether the operation is allowed.
    pub allow: bool,
    /// Denial reason or note.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reason: Option<String>,
    /// Row-level filter.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub row_filter: Option<tesela_ir::Filter>,
    /// Properties to redact.
    #[serde(default)]
    pub redactions: Vec<ApiName>,
    /// Whether explicit host approval is required before execution.
    #[serde(default)]
    pub require_approval: bool,
    /// Additional predicates the action input or subject must satisfy.
    #[serde(default)]
    pub input_constraints: Vec<tesela_ir::Filter>,
    /// Host-requested field replacements applied before validation/execution.
    #[serde(default)]
    pub transformations: BTreeMap<ApiName, Value>,
    /// Machine-readable obligations for the host.
    #[serde(default)]
    pub obligations: BTreeMap<String, Value>,
}

/// Backend capability advertisement.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BackendCapabilities {
    /// Supports search.
    pub search: bool,
    /// Supports get.
    pub get: bool,
    /// Supports create.
    pub create: bool,
    /// Supports update.
    pub update: bool,
    /// Supports delete.
    pub delete: bool,
    /// Supports aggregate.
    pub aggregate: bool,
    /// Supports native atomic upsert.
    pub upsert: bool,
    /// Supports native atomic batches.
    pub batch: bool,
    /// Supports conditional revision checks.
    pub conditional_mutation: bool,
}

#[cfg(test)]
mod tests {
    use super::{Query, TraversalQuery};

    #[test]
    fn query_rejects_a_stale_offset_key() {
        assert!(serde_json::from_str::<Query>(r#"{"offset":3}"#).is_err());
        assert!(serde_json::from_str::<Query>(r#"{"limit":3}"#).is_ok());
    }

    #[test]
    fn traversal_query_rejects_a_stale_offset_key() {
        assert!(serde_json::from_str::<TraversalQuery>(r#"{"source_pk":1,"offset":3}"#).is_err());
        assert!(serde_json::from_str::<TraversalQuery>(r#"{"source_pk":1,"limit":3}"#).is_ok());
    }
}
