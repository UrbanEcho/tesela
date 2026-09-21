//! Domain enums for Tesela ontology concepts.
//!
//! Every enum in this module is strongly typed. No magic strings are used
//! where a typed variant is possible.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Data types supported by Tesela properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataType {
    /// Variable-length UTF-8 string.
    String,
    /// Signed 32-bit integer.
    Integer,
    /// Signed 64-bit integer.
    #[serde(rename = "bigint")]
    BigInt,
    /// IEEE-754 double-precision float.
    Float,
    /// Decimal with arbitrary precision.
    Decimal,
    /// Boolean true/false.
    Boolean,
    /// Calendar date (YYYY-MM-DD).
    Date,
    /// Timestamp without time zone.
    Timestamp,
    /// Timestamp with time zone.
    #[serde(rename = "timestamptz")]
    TimestampTz,
    /// Universally unique identifier.
    Uuid,
    /// Arbitrary JSON value.
    Json,
    /// Geometric data (points, polygons, etc.).
    Geometry,
    /// Homogeneous array of another data type.
    Array,
    /// Enumeration with a fixed set of allowed values.
    Enum,
    /// Fixed-dimension floating-point vector for semantic / ANN search.
    ///
    /// The inner value is the embedding dimension (e.g. 1536 for `text-embedding-3-small`).
    Vector(u32),
}

/// Filter comparison operators for query predicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    /// Equality.
    Eq,
    /// Inequality.
    Ne,
    /// Membership in a set.
    In,
    /// Non-membership in a set.
    NotIn,
    /// Less than.
    Lt,
    /// Greater than.
    Gt,
    /// Less than or equal.
    Lte,
    /// Greater than or equal.
    Gte,
    /// SQL-style LIKE pattern match.
    Like,
    /// Range check (inclusive).
    Between,
    /// Substring containment.
    Contains,
    /// Prefix match.
    StartsWith,
    /// Logical AND of sub-filters.
    And,
    /// Logical OR of sub-filters.
    Or,
    /// Logical negation of a sub-filter.
    Not,
    /// IS NULL check.
    IsNull,
    /// IS NOT NULL check.
    IsNotNull,
}

impl FilterOp {
    /// Whether this operator is a logical operator (And, Or, Not).
    pub fn is_logical(&self) -> bool {
        matches!(self, FilterOp::And | FilterOp::Or | FilterOp::Not)
    }

    /// Whether this operator is a leaf scalar operator.
    pub fn is_scalar(&self) -> bool {
        !self.is_logical()
    }
}

/// Operations that can be performed on Tesela resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// Search / list records.
    Search,
    /// Read a single record.
    Read,
    /// Create, update, or delete.
    Mutate,
    /// Follow a link type.
    Traverse,
    /// Aggregation query.
    Aggregate,
    /// Bulk upload / ingestion.
    Upload,
    /// Execute an action.
    Execute,
}

/// Cardinality of a link type between two object types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkCardinality {
    /// One-to-one relationship.
    OneToOne,
    /// One-to-many relationship.
    OneToMany,
    /// Many-to-many relationship (requires junction table).
    ManyToMany,
}

/// Risk classification for actions.
///
/// Defaults to [`RiskLevel::Low`], so an action that declares nothing is not
/// silently treated as dangerous; the *effects* of an action are what gate
/// approval, and those default conservatively.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    /// Low risk — routine operation.
    #[default]
    Low,
    /// Medium risk — requires care.
    Medium,
    /// High risk — requires approval.
    High,
}

impl fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RiskLevel::Low => write!(f, "low"),
            RiskLevel::Medium => write!(f, "medium"),
            RiskLevel::High => write!(f, "high"),
        }
    }
}

impl FromStr for RiskLevel {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            _ => Err(crate::Error::validation(format!(
                "unknown risk level: {}",
                s
            ))),
        }
    }
}
