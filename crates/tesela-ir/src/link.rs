//! Link type definitions.

use serde::{Deserialize, Serialize};
use tesela_core::{ApiName, LinkCardinality};

/// A named, directional relationship between two object types.
///
/// A link type says *what* relates to *what*. It deliberately says nothing
/// about how the relation is stored: there is no source mapping, no join column
/// and no junction table here. A host binds a link to a backend resolver by
/// link id, so the same link can be served by a foreign key in one deployment
/// and an HTTP endpoint in another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkType {
    /// API name.
    pub api_name: ApiName,
    /// Display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// Source object type.
    pub from: ApiName,
    /// Target object type.
    pub to: ApiName,
    /// Cardinality.
    pub cardinality: LinkCardinality,
    /// The link that reads in the opposite direction, when one is declared.
    ///
    /// This is a semantic statement — "`owns` is the inverse of `owned_by`" —
    /// that lets a planner answer a reverse traversal without the caller
    /// having to know which direction was declared first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inverse: Option<ApiName>,
    /// When this link type was deprecated, as an ISO-8601 instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated_at: Option<String>,
}
