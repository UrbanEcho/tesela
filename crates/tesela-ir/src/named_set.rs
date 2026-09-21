//! Indexed collections of named ontology entities.

use std::collections::HashMap;
use std::ops::{Index, IndexMut};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tesela_core::ApiName;

/// An ontology entity that is addressable by API name.
pub trait SpecItem {
    /// The entity's API name.
    fn api_name(&self) -> &ApiName;
}

/// A declaration-ordered collection of [`SpecItem`]s with a name index.
///
/// Every ontology collection needs the same three operations — look one entity
/// up by name, replace it in place, remove it — and all three must be cheap.
/// The previous design re-scanned the whole `Vec` for each `upsert`, which is
/// what made the Python `spec.add` path quadratic; this type keeps a
/// `name → position` map instead.
///
/// The index is a *cache*, never a source of truth: lookups fall back to a scan
/// and repair the entry they were looking for, so a stale index can cost time
/// but can never return a wrong answer. That is also why the type deliberately
/// implements no `Deref` — a caller mutating through deref could rename an
/// entity behind the index's back, and shadowing `slice::get` would be a
/// silent footgun. Mutation goes through `IndexMut` or [`NamedSet::iter_mut`],
/// both of which invalidate the cache.
///
/// Serialization is a plain JSON array, so the wire format is unchanged from an
/// unindexed `Vec`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedSet<T> {
    items: Vec<T>,
    /// `name → position in items`. May be incomplete or stale; see the type
    /// documentation.
    index: HashMap<ApiName, usize>,
}

impl<T> NamedSet<T> {
    /// Create an empty set.
    #[must_use]
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            index: HashMap::new(),
        }
    }

    /// The entities in declaration order.
    #[must_use]
    pub fn as_slice(&self) -> &[T] {
        &self.items
    }

    /// The entities in declaration order.
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.items.iter()
    }

    /// The number of entities.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the set is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Iterate mutably, invalidating the name index.
    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.index.clear();
        self.items.iter_mut()
    }

    /// Drop every entity.
    pub fn clear(&mut self) {
        self.items.clear();
        self.index.clear();
    }
}

impl<T: SpecItem> NamedSet<T> {
    /// Rebuild the name index from scratch.
    ///
    /// The first declaration of a duplicated name wins, matching
    /// [`NamedSet::upsert`]'s replace-in-place semantics. Duplicates are a
    /// compile error, not something this container resolves.
    fn reindex(&mut self) {
        self.index.clear();
        for (position, item) in self.items.iter().enumerate() {
            self.index
                .entry(item.api_name().clone())
                .or_insert(position);
        }
    }

    /// Build a set from entities in declaration order.
    #[must_use]
    pub fn from_items(items: Vec<T>) -> Self {
        let mut set = Self {
            items,
            index: HashMap::new(),
        };
        set.reindex();
        set
    }

    /// Look an entity up by API name.
    ///
    /// Borrows the name, so no `ApiName` is allocated on the lookup path.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&T> {
        if let Some(&position) = self.index.get(name)
            && let Some(item) = self.items.get(position)
            && item.api_name() == name
        {
            return Some(item);
        }
        self.items.iter().find(|item| item.api_name() == name)
    }

    /// Whether an entity with this API name exists.
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// The position of an entity, repairing its index entry if it is stale.
    fn position(&mut self, name: &ApiName) -> Option<usize> {
        if let Some(&position) = self.index.get(name)
            && let Some(item) = self.items.get(position)
            && item.api_name() == name
        {
            return Some(position);
        }
        let found = self.items.iter().position(|item| item.api_name() == name)?;
        self.index
            .insert(self.items[found].api_name().clone(), found);
        Some(found)
    }

    /// Add an entity, replacing any existing entity with the same API name.
    ///
    /// Replacement keeps the original position, so declaration order stays
    /// stable across edits.
    pub fn upsert(&mut self, item: T) {
        let name = item.api_name().clone();
        match self.position(&name) {
            Some(position) => self.items[position] = item,
            None => {
                self.index.insert(name, self.items.len());
                self.items.push(item);
            }
        }
    }

    /// Append an entity.
    ///
    /// A duplicate name is appended rather than replacing; the compiler reports
    /// it as a diagnostic so the author sees the mistake.
    pub fn push(&mut self, item: T) {
        let name = item.api_name().clone();
        if self.get(&name).is_none() {
            self.index.insert(name, self.items.len());
        }
        self.items.push(item);
    }

    /// Remove an entity by API name, reporting whether it was present.
    pub fn remove(&mut self, name: &ApiName) -> bool {
        let Some(position) = self.position(name) else {
            return false;
        };
        self.items.remove(position);
        self.reindex();
        true
    }

    /// API names declared more than once, in declaration order.
    #[must_use]
    pub fn duplicates(&self) -> Vec<ApiName> {
        let mut seen = HashMap::with_capacity(self.items.len());
        let mut duplicates = Vec::new();
        for item in &self.items {
            let name = item.api_name();
            if seen.insert(name.clone(), ()).is_some() {
                duplicates.push(name.clone());
            }
        }
        duplicates
    }
}

impl<T> Default for NamedSet<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: SpecItem> Index<usize> for NamedSet<T> {
    type Output = T;

    fn index(&self, position: usize) -> &Self::Output {
        &self.items[position]
    }
}

impl<T: SpecItem> IndexMut<usize> for NamedSet<T> {
    fn index_mut(&mut self, position: usize) -> &mut Self::Output {
        self.index.clear();
        &mut self.items[position]
    }
}

impl<'a, T> IntoIterator for &'a NamedSet<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl<T: SpecItem> Serialize for NamedSet<T>
where
    T: Serialize,
{
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.items.serialize(serializer)
    }
}

impl<'de, T> Deserialize<'de> for NamedSet<T>
where
    T: SpecItem + Deserialize<'de>,
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::from_items(Vec::<T>::deserialize(deserializer)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Entity {
        api_name: ApiName,
        label: String,
    }

    impl SpecItem for Entity {
        fn api_name(&self) -> &ApiName {
            &self.api_name
        }
    }

    fn entity(name: &'static str, label: &str) -> Entity {
        Entity {
            api_name: ApiName::from(name),
            label: label.to_string(),
        }
    }

    #[test]
    fn upsert_replaces_in_place_and_keeps_declaration_order() {
        let mut set = NamedSet::from_items(vec![entity("a", "first"), entity("b", "second")]);
        set.upsert(entity("a", "replaced"));
        assert_eq!(set.len(), 2);
        assert_eq!(set[0].label, "replaced");
        assert_eq!(set[1].api_name.as_ref(), "b");
    }

    #[test]
    fn upsert_appends_an_unknown_name() {
        let mut set = NamedSet::new();
        set.upsert(entity("a", "first"));
        assert_eq!(set.get("a").map(|item| item.label.as_str()), Some("first"));
    }

    #[test]
    fn lookups_survive_mutation_through_the_index() {
        let mut set = NamedSet::from_items(vec![entity("a", "first"), entity("b", "second")]);
        set[1].api_name = ApiName::from("renamed");
        assert!(set.get("b").is_none());
        assert_eq!(
            set.get("renamed").map(|item| item.label.as_str()),
            Some("second")
        );
        // The repaired index now serves the renamed entity.
        assert!(set.contains("renamed"));
    }

    #[test]
    fn removal_shifts_the_index() {
        let mut set = NamedSet::from_items(vec![
            entity("a", "first"),
            entity("b", "second"),
            entity("c", "third"),
        ]);
        assert!(set.remove(&ApiName::from("a")));
        assert!(!set.remove(&ApiName::from("a")));
        assert_eq!(set.len(), 2);
        assert_eq!(set.get("c").map(|item| item.label.as_str()), Some("third"));
        assert_eq!(set[0].api_name.as_ref(), "b");
    }

    #[test]
    fn duplicates_are_reported_not_hidden() {
        let mut set = NamedSet::from_items(vec![entity("a", "first")]);
        set.push(entity("a", "second"));
        assert_eq!(set.len(), 2);
        assert_eq!(set.duplicates(), vec![ApiName::from("a")]);
        // The index resolves the first declaration.
        assert_eq!(set.get("a").map(|item| item.label.as_str()), Some("first"));
    }

    #[test]
    fn serialization_is_a_plain_array() {
        let set = NamedSet::from_items(vec![entity("a", "first")]);
        let json = serde_json::to_string(&set).expect("set should serialize");
        assert_eq!(json, r#"[{"api_name":"a","label":"first"}]"#);
        let parsed: NamedSet<Entity> = serde_json::from_str(&json).expect("set should deserialize");
        assert_eq!(parsed, set);
        // Deserialization rebuilds the index.
        assert!(parsed.contains("a"));
    }
}
