//! Records and result types exchanged with backends.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use tesela_core::{ApiName, Error, Key, Value};

/// An opaque, backend-defined version token for a record.
///
/// Tesela never interprets a revision — it may be a monotonic counter, an
/// ETag, or a timestamp. It is carried so a write can state which version it
/// read, and so a host can detect a lost update.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(Arc<str>);

impl Revision {
    /// Create a revision from a backend-supplied token.
    pub fn new<S: AsRef<str>>(token: S) -> Self {
        Self(Arc::from(token.as_ref()))
    }

    /// The token as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Revision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for Revision {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for Revision {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl Serialize for Revision {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Revision {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let token = String::deserialize(deserializer)?;
        Ok(Self::new(token))
    }
}

/// A single record.
///
/// A record always has a primary key: a row without one cannot be addressed,
/// and modelling that as `Option` pushed the same "is this record addressable?"
/// question into every caller, filter and set operation. The key is a [`Key`]
/// rather than a [`Value`] because an identity must be hashable, totally
/// ordered and allocation-free to compare.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// Primary key.
    pub primary_key: Key,
    /// Field values keyed by property API name.
    #[serde(default)]
    pub values: BTreeMap<ApiName, Value>,
    /// The backend's version token for this record, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<Revision>,
}

impl Record {
    /// Create a record from a primary key and its values.
    pub fn new(primary_key: Key, values: BTreeMap<ApiName, Value>) -> Self {
        Self {
            primary_key,
            values,
            revision: None,
        }
    }

    /// Return a field value by name.
    ///
    /// Borrows the name: `ApiName` implements `Borrow<str>`, so this performs no
    /// allocation and no `Arc` clone.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    /// Return a string field.
    #[must_use]
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    /// Return a field as JSON.
    #[must_use]
    pub fn get_json(&self, key: &str) -> Option<serde_json::Value> {
        self.get(key).cloned().map(serde_json::Value::from)
    }

    /// Decode a field into a typed value.
    pub fn decode<T: DeserializeOwned>(&self, key: &str) -> Result<T, Error> {
        let json = self
            .get_json(key)
            .ok_or_else(|| Error::bad_request(format!("{key} is missing")))?;
        serde_json::from_value(json).map_err(|error| Error::bad_request(error.to_string()))
    }

    /// Consume the record into a JSON object of its field values.
    ///
    /// The primary key and revision are not included: this is the *values*
    /// projection a backend writes, and the key is carried alongside it.
    #[must_use]
    pub fn into_json_map(self) -> serde_json::Map<String, serde_json::Value> {
        self.values
            .into_iter()
            .map(|(key, value)| (key.to_string(), serde_json::Value::from(value)))
            .collect()
    }
}

impl From<Record> for serde_json::Value {
    fn from(record: Record) -> Self {
        serde_json::Value::Object(record.into_json_map())
    }
}

/// A page of records.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Page {
    /// Records in this page.
    #[serde(default)]
    pub records: Vec<Record>,
    /// Opaque cursor for the next page, when one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Result of a mutation.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MutationResult {
    /// The record as stored, when the backend can return it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record: Option<Record>,
    /// Number of affected rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows_affected: Option<i64>,
}

/// Result of an action execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionResult {
    /// Outcome: `success`, `failed`, `rejected` or `queued`.
    pub status: String,
    /// Output payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    /// Failure detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Identifier of the run that produced this result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

/// Result of an aggregate query.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AggregateResult {
    /// One entry per group, mapping group keys and aggregate aliases to values.
    #[serde(default)]
    pub groups: Vec<BTreeMap<String, Value>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> Record {
        let mut values = BTreeMap::new();
        values.insert(ApiName::from("tenant_id"), Value::string("acme"));
        Record::new(Key::from(&Value::string("cust_1")), values)
    }

    #[test]
    fn lookups_by_name_do_not_allocate() {
        let record = record();
        assert_eq!(record.get_str("tenant_id"), Some("acme"));
        assert!(record.get("missing").is_none());
    }

    #[test]
    fn a_record_round_trips_with_its_revision() {
        let mut original = record();
        original.revision = Some(Revision::new("etag-7"));
        let json = serde_json::to_string(&original).expect("record serializes");
        let parsed: Record = serde_json::from_str(&json).expect("record deserializes");
        assert_eq!(parsed, original);
        assert_eq!(
            parsed.revision.as_ref().map(Revision::as_str),
            Some("etag-7")
        );
    }

    #[test]
    fn the_primary_key_is_flat_on_the_wire() {
        let json = serde_json::to_string(&record()).expect("record serializes");
        assert!(json.starts_with(r#"{"primary_key":"cust_1""#), "got {json}");
    }
}
