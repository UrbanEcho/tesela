//! Semantic diffs between ontology specifications.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tesela_ir::Spec;

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
