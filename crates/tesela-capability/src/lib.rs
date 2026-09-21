#![deny(warnings)]
#![deny(missing_docs)]

//! Actor-specific semantic capability projection and bounded result handles.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tesela_core::{ApiName, Error, ExecutionContext, Key, Operation, RiskLevel, Value};
use tesela_ir::{ActionEffects, Concurrency, Filter, Record, SemanticFingerprint, Spec};

/// How a capability may be exposed to a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExposure {
    /// Included in the initial tool surface.
    Direct,
    /// Returned through deferred search.
    Discoverable,
    /// Available only to a future compiled program surface.
    ProgramOnly,
    /// Not callable by the principal.
    Hidden,
}

/// One ontology-derived operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    /// Stable qualified name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Semantic operation executed by this capability.
    pub operation: Operation,
    /// Exposure selected by the compiler.
    pub exposure: ToolExposure,
    /// JSON-schema-shaped input contract.
    pub input_schema: Value,
    /// Declared action effects; read capabilities use `read_only`.
    pub effects: ActionEffects,
    /// Explicit approval requirement.
    pub approval_required: bool,
    /// Static action risk.
    pub risk: RiskLevel,
    /// Whether repeating the same invocation is declared safe.
    pub idempotent: bool,
    /// Host concurrency contract for actions.
    pub concurrency: Option<Concurrency>,
    /// Whether a subject revision must accompany the call.
    pub requires_revision: bool,
    /// Semantic predicates checked immediately before execution.
    pub preconditions: Vec<Filter>,
}

/// Versioned capability surface compiled for one principal and task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityCatalog {
    /// Ontology generation.
    pub generation: u64,
    /// Semantic identity.
    pub fingerprint: SemanticFingerprint,
    /// Projected operations.
    pub capabilities: Vec<Capability>,
}

impl CapabilityCatalog {
    /// Search discoverable capabilities. Exact and qualified names always rank first.
    #[must_use]
    pub fn search(&self, query: &str, limit: usize) -> Vec<&Capability> {
        let query = query.to_ascii_lowercase();
        let terms: Vec<&str> = query
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .filter(|term| !term.is_empty())
            .collect();
        let mut scored: Vec<_> = self
            .capabilities
            .iter()
            .filter(|capability| capability.exposure != ToolExposure::Hidden)
            .map(|capability| {
                let name = capability.name.to_ascii_lowercase();
                let text = format!("{name} {}", capability.description.to_ascii_lowercase());
                let exact =
                    usize::from(name == query || name.rsplit('.').next() == Some(query.as_str()));
                let lexical: usize = terms.iter().map(|term| text.matches(term).count()).sum();
                (exact, lexical, capability)
            })
            .filter(|(exact, lexical, _)| *exact > 0 || *lexical > 0)
            .collect();
        scored.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| right.1.cmp(&left.1))
                .then_with(|| left.2.name.cmp(&right.2.name))
        });
        scored
            .into_iter()
            .take(limit)
            .map(|(_, _, capability)| capability)
            .collect()
    }
}

/// Invocation identity used to reject stale calls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityInvocation {
    /// Catalog generation presented to the caller.
    pub generation: u64,
    /// Catalog fingerprint presented to the caller.
    pub fingerprint: SemanticFingerprint,
    /// Selected capability.
    pub capability: String,
    /// Input payload.
    pub input: Value,
}

/// Bounded execution-scoped records retained outside model context.
#[derive(Debug, Clone)]
pub struct ResultHandle {
    id: Arc<str>,
    request_id: Arc<str>,
    records: Arc<[Record]>,
}

impl ResultHandle {
    /// Create a bounded handle.
    pub fn new(
        context: &ExecutionContext,
        id: impl AsRef<str>,
        records: Vec<Record>,
        max_rows: usize,
    ) -> Result<Self, Error> {
        if records.len() > max_rows {
            return Err(Error::budget_exhausted("result handle row budget exceeded"));
        }
        context.charge_rows(records.len() as u64)?;
        Ok(Self {
            id: Arc::from(id.as_ref()),
            request_id: Arc::from(context.request_id()),
            records: records.into(),
        })
    }
    /// Handle identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Page retained records.
    pub fn page(
        &self,
        context: &ExecutionContext,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Record>, Error> {
        self.check_context(context)?;
        Ok(self
            .records
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect())
    }
    /// Project selected fields.
    pub fn project(
        &self,
        context: &ExecutionContext,
        fields: &[ApiName],
    ) -> Result<Vec<Record>, Error> {
        self.check_context(context)?;
        Ok(self
            .records
            .iter()
            .cloned()
            .map(|mut record| {
                record.values.retain(|field, _| fields.contains(field));
                record
            })
            .collect())
    }
    /// Filter retained records with a caller-compiled predicate.
    pub fn filter(
        &self,
        context: &ExecutionContext,
        mut predicate: impl FnMut(&Record) -> bool,
    ) -> Result<Vec<Record>, Error> {
        self.check_context(context)?;
        Ok(self
            .records
            .iter()
            .filter(|record| predicate(record))
            .cloned()
            .collect())
    }
    /// Count rows by a field value.
    pub fn count_by(
        &self,
        context: &ExecutionContext,
        field: &ApiName,
    ) -> Result<BTreeMap<Key, u64>, Error> {
        self.check_context(context)?;
        let mut counts = BTreeMap::new();
        for record in self.records.iter() {
            if let Some(value) = record.values.get(field) {
                *counts.entry(Key::from(value)).or_default() += 1;
            }
        }
        Ok(counts)
    }

    fn check_context(&self, context: &ExecutionContext) -> Result<(), Error> {
        if self.request_id.as_ref() == context.request_id() {
            Ok(())
        } else {
            Err(Error::unauthorized(
                "result handle belongs to a different execution context",
            ))
        }
    }
}

/// Semantic impact category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffImpact {
    /// Additive or reader-compatible change.
    Safe,
    /// Existing callers must change.
    Breaking,
    /// Authority or policy meaning changed.
    Security,
    /// Cached capability catalogs must be rebuilt.
    AgentCatalog,
    /// Host bindings may need a new capability.
    BackendCapability,
}

/// One semantic change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticChange {
    /// Impact class.
    pub impact: DiffImpact,
    /// Stable entity path.
    pub path: String,
    /// Explanation.
    pub description: String,
}

/// Compare two semantic specifications.
#[must_use]
pub fn semantic_diff(before: &Spec, after: &Spec) -> Vec<SemanticChange> {
    let old: HashMap<_, _> = before
        .object_types
        .iter()
        .map(|item| (item.api_name.as_str(), item))
        .collect();
    let new: HashMap<_, _> = after
        .object_types
        .iter()
        .map(|item| (item.api_name.as_str(), item))
        .collect();
    let mut changes = Vec::new();
    for name in old.keys().filter(|name| !new.contains_key(*name)) {
        changes.push(SemanticChange {
            impact: DiffImpact::Breaking,
            path: format!("object_types.{name}"),
            description: "object type removed".to_string(),
        });
    }
    for name in new.keys().filter(|name| !old.contains_key(*name)) {
        changes.push(SemanticChange {
            impact: DiffImpact::Safe,
            path: format!("object_types.{name}"),
            description: "object type added".to_string(),
        });
    }
    for (name, old_object) in &old {
        let Some(new_object) = new.get(name) else {
            continue;
        };
        let old_properties: HashMap<_, _> = old_object
            .properties
            .iter()
            .map(|property| (property.api_name.as_str(), property))
            .collect();
        let new_properties: HashMap<_, _> = new_object
            .properties
            .iter()
            .map(|property| (property.api_name.as_str(), property))
            .collect();
        for property in old_properties
            .keys()
            .filter(|property| !new_properties.contains_key(*property))
        {
            changes.push(SemanticChange {
                impact: DiffImpact::Breaking,
                path: format!("object_types.{name}.properties.{property}"),
                description: "property removed".to_string(),
            });
        }
        for property in new_properties
            .keys()
            .filter(|property| !old_properties.contains_key(*property))
        {
            let definition = new_properties[property];
            changes.push(SemanticChange {
                impact: if definition.nullable.unwrap_or(false) || definition.default.is_some() {
                    DiffImpact::Safe
                } else {
                    DiffImpact::Breaking
                },
                path: format!("object_types.{name}.properties.{property}"),
                description: "property added".to_string(),
            });
        }
        for (property, old_property) in old_properties {
            if new_properties
                .get(property)
                .is_some_and(|new_property| *new_property != old_property)
            {
                changes.push(SemanticChange {
                    impact: DiffImpact::Breaking,
                    path: format!("object_types.{name}.properties.{property}"),
                    description: "property contract changed".to_string(),
                });
            }
        }
        if old_object.primary_key != new_object.primary_key {
            changes.push(SemanticChange {
                impact: DiffImpact::BackendCapability,
                path: format!("object_types.{name}.primary_key"),
                description: "primary key changed; bindings must be revalidated".to_string(),
            });
        }
    }
    let old_links: HashMap<_, _> = before
        .link_types
        .iter()
        .map(|item| (item.api_name.as_str(), item))
        .collect();
    let new_links: HashMap<_, _> = after
        .link_types
        .iter()
        .map(|item| (item.api_name.as_str(), item))
        .collect();
    for name in old_links
        .keys()
        .filter(|name| !new_links.contains_key(*name))
    {
        changes.push(SemanticChange {
            impact: DiffImpact::Breaking,
            path: format!("link_types.{name}"),
            description: "link removed".to_string(),
        });
    }
    for name in new_links
        .keys()
        .filter(|name| !old_links.contains_key(*name))
    {
        changes.push(SemanticChange {
            impact: DiffImpact::BackendCapability,
            path: format!("link_types.{name}"),
            description: "link added; a resolver may be required".to_string(),
        });
    }
    let old_actions: HashMap<_, _> = before
        .actions
        .iter()
        .map(|item| (item.api_name.as_str(), item))
        .collect();
    let new_actions: HashMap<_, _> = after
        .actions
        .iter()
        .map(|item| (item.api_name.as_str(), item))
        .collect();
    for name in old_actions
        .keys()
        .filter(|name| !new_actions.contains_key(*name))
    {
        changes.push(SemanticChange {
            impact: DiffImpact::AgentCatalog,
            path: format!("actions.{name}"),
            description: "action removed; cached catalogs must be invalidated".to_string(),
        });
    }
    for name in new_actions
        .keys()
        .filter(|name| !old_actions.contains_key(*name))
    {
        changes.push(SemanticChange {
            impact: DiffImpact::AgentCatalog,
            path: format!("actions.{name}"),
            description: "action added to capability projection".to_string(),
        });
    }
    for (name, old_action) in &old_actions {
        let Some(new_action) = new_actions.get(name) else {
            continue;
        };
        if old_action.effects != new_action.effects
            || old_action.approval != new_action.approval
            || old_action.requires_revision != new_action.requires_revision
            || old_action.preconditions != new_action.preconditions
        {
            changes.push(SemanticChange {
                impact: DiffImpact::Security,
                path: format!("actions.{name}.execution_contract"),
                description: "effects, approval, revision, or preconditions changed".to_string(),
            });
        }
        if old_action.input_schema != new_action.input_schema
            || old_action.output_schema != new_action.output_schema
        {
            changes.push(SemanticChange {
                impact: DiffImpact::Breaking,
                path: format!("actions.{name}.schema"),
                description: "action input or output schema changed".to_string(),
            });
        }
    }
    changes.sort_by(|left, right| left.path.cmp(&right.path));
    changes
}

#[cfg(test)]
mod tests;
