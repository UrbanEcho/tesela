//! The semantic ontology document.
//!
//! A [`Spec`] is a *semantic* document: it describes the entities, properties,
//! links and actions a platform exposes, and says nothing about where the data
//! lives. Physical mapping — repositories, source columns, junction tables,
//! indexes — is a host concern that belongs to the backend bindings, not to a
//! portable ontology. Removing it is what makes the same spec runnable against
//! an in-memory backend, a Postgres adapter and an HTTP API without edits.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sha2::Digest;
use tesela_core::{ApiName, Error, Version};

use crate::named_set::{NamedSet, SpecItem};
use crate::{ActionType, LinkType, ObjectType, Trait};

/// Current spec version string.
pub const SPEC_VERSION: &str = "tesela.spec.v1";

/// Version of the compilation rules applied to a [`Spec`].
///
/// Bumping this changes every [`SemanticFingerprint`], which is how a host
/// detects that a cached compilation or a cached tool catalog was produced by
/// older rules and must be rebuilt.
pub const COMPILER_VERSION: u32 = 1;

/// The root ontology document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spec {
    /// Spec version identifier.
    pub version: Version,
    /// Reusable trait definitions.
    #[serde(default, skip_serializing_if = "NamedSet::is_empty")]
    pub traits: NamedSet<Trait>,
    /// Object type definitions.
    #[serde(default, skip_serializing_if = "NamedSet::is_empty")]
    pub object_types: NamedSet<ObjectType>,
    /// Link type definitions.
    #[serde(default, skip_serializing_if = "NamedSet::is_empty")]
    pub link_types: NamedSet<LinkType>,
    /// Action type definitions.
    #[serde(default, skip_serializing_if = "NamedSet::is_empty")]
    pub actions: NamedSet<ActionType>,
}

impl Spec {
    /// Parse a spec from JSON bytes.
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        serde_json::from_slice(data)
            .map_err(|error| Error::validation(format!("invalid spec JSON: {error}")))
    }

    /// Serialize to compact JSON bytes.
    pub fn to_json(&self) -> Result<Vec<u8>, Error> {
        serde_json::to_vec(self)
            .map_err(|error| Error::internal(format!("JSON serialization failed: {error}")))
    }

    /// Serialize to compact JSON string.
    pub fn to_json_string(&self) -> Result<String, Error> {
        serde_json::to_string(self)
            .map_err(|error| Error::internal(format!("JSON serialization failed: {error}")))
    }

    /// Serialize to pretty JSON.
    pub fn to_json_pretty(&self) -> Result<String, Error> {
        serde_json::to_string_pretty(self)
            .map_err(|error| Error::internal(format!("JSON serialization failed: {error}")))
    }

    /// Compute the spec's [`SemanticFingerprint`].
    ///
    /// The fingerprint identifies the *meaning* of the spec, not its byte
    /// layout: entities are canonically ordered before hashing, so two specs
    /// that declare the same ontology in a different order fingerprint
    /// identically. It covers the spec version and [`COMPILER_VERSION`], so a
    /// change in either is a different fingerprint.
    pub fn fingerprint(&self) -> Result<SemanticFingerprint, Error> {
        let canonical = canonicalize(
            serde_json::to_value(self)
                .map_err(|error| Error::internal(format!("spec serialization failed: {error}")))?,
        );
        let bytes = serde_json::to_vec(&canonical)
            .map_err(|error| Error::internal(format!("spec serialization failed: {error}")))?;
        let mut hasher = sha2::Sha256::new();
        hasher.update(SPEC_VERSION.as_bytes());
        hasher.update([0]);
        hasher.update(COMPILER_VERSION.to_le_bytes());
        hasher.update(&bytes);
        Ok(SemanticFingerprint {
            digest: hasher.finalize().into(),
            spec_version: self.version.clone(),
            compiler_version: COMPILER_VERSION,
        })
    }

    /// Add or replace an object type by API name.
    pub fn upsert_object_type(&mut self, item: ObjectType) {
        self.object_types.upsert(item);
    }

    /// Add or replace a link type by API name.
    pub fn upsert_link_type(&mut self, item: LinkType) {
        self.link_types.upsert(item);
    }

    /// Add or replace an action by API name.
    pub fn upsert_action(&mut self, item: ActionType) {
        self.actions.upsert(item);
    }

    /// Add or replace a trait by API name.
    pub fn upsert_trait(&mut self, item: Trait) {
        self.traits.upsert(item);
    }

    /// Remove an entity by kind and API name.
    ///
    /// The kind is a typed [`EntityKind`] rather than a string: the previous
    /// `&str` form silently returned `false` for a misspelt kind, which is
    /// indistinguishable from "not found".
    pub fn remove(&mut self, kind: EntityKind, api_name: &ApiName) -> bool {
        match kind {
            EntityKind::Trait => self.traits.remove(api_name),
            EntityKind::ObjectType => self.object_types.remove(api_name),
            EntityKind::LinkType => self.link_types.remove(api_name),
            EntityKind::Action => self.actions.remove(api_name),
        }
    }
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            version: Version::new(SPEC_VERSION),
            traits: NamedSet::new(),
            object_types: NamedSet::new(),
            link_types: NamedSet::new(),
            actions: NamedSet::new(),
        }
    }
}

/// The kinds of entity a [`Spec`] declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    /// A reusable property bundle.
    Trait,
    /// A domain entity type.
    ObjectType,
    /// A relationship between two object types.
    LinkType,
    /// A named, executable operation.
    Action,
}

impl EntityKind {
    /// The kind's stable name, for diagnostics and tool output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trait => "trait",
            Self::ObjectType => "object_type",
            Self::LinkType => "link_type",
            Self::Action => "action",
        }
    }

    /// Every entity kind, in declaration order.
    pub const ALL: &'static [Self] = &[Self::Trait, Self::ObjectType, Self::LinkType, Self::Action];
}

impl std::fmt::Display for EntityKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A content-addressed identity for the *meaning* of a spec.
///
/// Two hosts that independently load the same ontology produce the same
/// fingerprint, which is what lets a caller detect that a tool catalog, a
/// cached compilation or a stored request was built against a different
/// ontology than the one now active.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SemanticFingerprint {
    digest: [u8; 32],
    spec_version: Version,
    compiler_version: u32,
}

impl SemanticFingerprint {
    /// The raw digest bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.digest
    }

    /// The digest as lowercase hexadecimal.
    #[must_use]
    pub fn to_hex(&self) -> String {
        hex::encode(self.digest)
    }

    /// The spec version this fingerprint covers.
    #[must_use]
    pub fn spec_version(&self) -> &Version {
        &self.spec_version
    }

    /// The compilation rules this fingerprint covers.
    #[must_use]
    pub fn compiler_version(&self) -> u32 {
        self.compiler_version
    }
}

impl std::fmt::Display for SemanticFingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for SemanticFingerprint {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("SemanticFingerprint", 3)?;
        state.serialize_field("digest", &self.to_hex())?;
        state.serialize_field("spec_version", self.spec_version.as_str())?;
        state.serialize_field("compiler_version", &self.compiler_version)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for SemanticFingerprint {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            digest: String,
            spec_version: String,
            compiler_version: u32,
        }
        let wire = Wire::deserialize(deserializer)?;
        let bytes = hex::decode(&wire.digest).map_err(serde::de::Error::custom)?;
        let digest: [u8; 32] = bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("fingerprint digest must be 32 bytes"))?;
        Ok(Self {
            digest,
            spec_version: Version::new(wire.spec_version),
            compiler_version: wire.compiler_version,
        })
    }
}

/// Rewrite a spec's JSON form into a canonical one.
///
/// Canonical here means one thing: **an array of objects that all carry an
/// `api_name` is sorted by that name.** Every ontology collection is such an
/// array, so declaration order stops affecting the fingerprint, while arrays
/// whose order is meaningful — allowed values, tags — are left alone. Object
/// keys are already sorted by `serde_json`'s map, so this is the only
/// normalization needed.
fn canonicalize(value: serde_json::Value) -> serde_json::Value {
    use serde_json::Value as Json;
    match value {
        Json::Array(items) => {
            let mut items: Vec<Json> = items.into_iter().map(canonicalize).collect();
            let named = !items.is_empty()
                && items
                    .iter()
                    .all(|item| item.get("api_name").is_some_and(Json::is_string));
            if named {
                items.sort_by(|left, right| {
                    let left = left
                        .get("api_name")
                        .and_then(Json::as_str)
                        .unwrap_or_default();
                    let right = right
                        .get("api_name")
                        .and_then(Json::as_str)
                        .unwrap_or_default();
                    left.cmp(right)
                });
            }
            Json::Array(items)
        }
        Json::Object(map) => Json::Object(
            map.into_iter()
                .map(|(key, value)| (key, canonicalize(value)))
                .collect(),
        ),
        other => other,
    }
}

/// Implement [`SpecItem`] for the entity types a spec can hold.
macro_rules! impl_spec_item {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl SpecItem for $ty {
                fn api_name(&self) -> &ApiName {
                    &self.api_name
                }
            }
        )+
    };
}

impl_spec_item!(Trait, ObjectType, LinkType, ActionType);

/// Count duplicated names across every collection, for compiler diagnostics.
///
/// Kept here rather than in the compiler so that a caller can report authoring
/// mistakes without running a full compile.
#[must_use]
pub fn duplicate_names(spec: &Spec) -> HashMap<EntityKind, Vec<ApiName>> {
    let mut found = HashMap::new();
    let collections = [
        (EntityKind::Trait, spec.traits.duplicates()),
        (EntityKind::ObjectType, spec.object_types.duplicates()),
        (EntityKind::LinkType, spec.link_types.duplicates()),
        (EntityKind::Action, spec.actions.duplicates()),
    ];
    for (kind, duplicates) in collections {
        if !duplicates.is_empty() {
            found.insert(kind, duplicates);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use tesela_core::{DataType, LinkCardinality, Value};

    fn property(name: &'static str) -> crate::Property {
        crate::Property {
            api_name: ApiName::from(name),
            display: None,
            description: None,
            data_type: DataType::String,
            nullable: None,
            default: None,
            allowed_values: None,
            tags: Vec::new(),
        }
    }

    fn object_type(name: &'static str) -> ObjectType {
        ObjectType {
            api_name: ApiName::from(name),
            display: None,
            description: None,
            primary_key: ApiName::from("id"),
            properties: vec![property("id")],
            traits: Vec::new(),
            tags: Vec::new(),
            deprecated_at: None,
        }
    }

    #[test]
    fn fingerprint_ignores_declaration_order() {
        let mut left = Spec::default();
        left.upsert_object_type(object_type("customer"));
        left.upsert_object_type(object_type("order"));

        let mut right = Spec::default();
        right.upsert_object_type(object_type("order"));
        right.upsert_object_type(object_type("customer"));

        assert_eq!(
            left.fingerprint().expect("fingerprint").to_hex(),
            right.fingerprint().expect("fingerprint").to_hex()
        );
    }

    #[test]
    fn fingerprint_changes_with_meaning() {
        let plain = Spec::default();
        let mut extended = Spec::default();
        extended.upsert_object_type(object_type("customer"));

        assert_ne!(
            plain.fingerprint().expect("fingerprint"),
            extended.fingerprint().expect("fingerprint")
        );
    }

    #[test]
    fn fingerprint_is_stable_across_serialization() {
        let mut spec = Spec::default();
        spec.upsert_object_type(object_type("customer"));
        let first = spec.fingerprint().expect("fingerprint");
        let round_tripped = Spec::parse(&spec.to_json().expect("spec should serialize"))
            .expect("spec should parse");
        assert_eq!(first, round_tripped.fingerprint().expect("fingerprint"));
    }

    #[test]
    fn fingerprint_round_trips_through_json() {
        let mut spec = Spec::default();
        spec.upsert_object_type(object_type("customer"));
        let fingerprint = spec.fingerprint().expect("fingerprint");
        let json = serde_json::to_string(&fingerprint).expect("fingerprint should serialize");
        assert!(json.contains(&fingerprint.to_hex()));
        let parsed: SemanticFingerprint =
            serde_json::from_str(&json).expect("fingerprint should deserialize");
        assert_eq!(parsed, fingerprint);
        assert_eq!(parsed.spec_version().as_str(), SPEC_VERSION);
        assert_eq!(parsed.compiler_version(), COMPILER_VERSION);
    }

    #[test]
    fn upsert_replaces_without_reordering() {
        let mut spec = Spec::default();
        spec.upsert_object_type(object_type("customer"));
        spec.upsert_object_type(object_type("order"));
        let mut replacement = object_type("customer");
        replacement.description = Some("updated".to_string());
        spec.upsert_object_type(replacement);
        assert_eq!(spec.object_types.len(), 2);
        assert_eq!(spec.object_types[0].api_name.as_ref(), "customer");
        assert_eq!(spec.object_types[0].description.as_deref(), Some("updated"));
    }

    #[test]
    fn remove_is_typed_by_kind() {
        let mut spec = Spec::default();
        spec.upsert_object_type(object_type("customer"));
        spec.upsert_link_type(LinkType {
            api_name: ApiName::from("owns"),
            display: None,
            from: ApiName::from("customer"),
            to: ApiName::from("order"),
            cardinality: LinkCardinality::OneToMany,
            inverse: None,
            deprecated_at: None,
        });
        assert!(spec.remove(EntityKind::ObjectType, &ApiName::from("customer")));
        assert!(!spec.remove(EntityKind::ObjectType, &ApiName::from("owns")));
        assert_eq!(spec.link_types.len(), 1);
    }

    #[test]
    fn duplicate_names_are_detected_per_kind() {
        let mut spec = Spec::default();
        spec.object_types.push(object_type("customer"));
        spec.object_types.push(object_type("customer"));
        let duplicates = duplicate_names(&spec);
        assert_eq!(
            duplicates.get(&EntityKind::ObjectType),
            Some(&vec![ApiName::from("customer")])
        );
    }

    #[test]
    fn empty_collections_are_omitted_from_the_wire_form() {
        let json = Spec::default().to_json_string().expect("spec serializes");
        assert_eq!(json, r#"{"version":"tesela.spec.v1"}"#);
    }

    #[test]
    fn canonicalize_leaves_ordered_arrays_alone() {
        let spec = serde_json::json!({
            "allowed_values": [{"b": 1}, {"a": 2}],
            "properties": [{"api_name": "z"}, {"api_name": "a"}],
        });
        let canonical = canonicalize(spec);
        assert_eq!(
            canonical["allowed_values"],
            serde_json::json!([{"b": 1}, {"a": 2}])
        );
        assert_eq!(canonical["properties"][0]["api_name"], "a");
    }

    #[test]
    fn allowed_values_survive_a_round_trip() {
        let mut spec = Spec::default();
        let mut customer = object_type("customer");
        customer.properties[0].allowed_values =
            Some(vec![Value::string("active"), Value::string("closed")]);
        spec.upsert_object_type(customer);
        let parsed = Spec::parse(&spec.to_json().expect("spec serializes")).expect("spec parses");
        assert_eq!(
            parsed.object_types[0].properties[0]
                .allowed_values
                .as_ref()
                .map(Vec::len),
            Some(2)
        );
    }
}
