//! Allocation-free comparison between [`Key`] and [`Value`], and a hash set of
//! keys that can be probed with a borrowed value.
//!
//! Filter evaluation compares a stored value against a compiled operand once
//! per scanned row. Projecting the stored value into a [`Key`] first allocates
//! for every string, so these helpers compare in place instead and agree with
//! `Key::from(value)` for every input.

use std::cmp::Ordering;

use serde_json::Value as Json;

use crate::key::{Bits, Key};
use crate::value::Value;

/// Variant rank of the key a JSON value projects to, in [`Key`] order.
fn value_rank(value: &Json) -> u8 {
    match value {
        Json::Null => 0,
        Json::Bool(_) => 1,
        Json::Number(number) => {
            if number.is_i64() {
                2
            } else if number.is_u64() {
                3
            } else {
                4
            }
        }
        Json::String(_) => 5,
        Json::Array(_) | Json::Object(_) => 6,
    }
}

const fn key_rank(key: &Key) -> u8 {
    match key {
        Key::Null => 0,
        Key::Bool(_) => 1,
        Key::I64(_) => 2,
        Key::U64(_) => 3,
        Key::F64(_) => 4,
        Key::Str(_) => 5,
        Key::Bytes(_) => 6,
    }
}

impl Key {
    /// Compare this key with the key `value` would project to, without
    /// allocating for scalars.
    ///
    /// Equivalent to `self.cmp(&Key::from(value))`. Arrays and objects have no
    /// scalar identity and fall back to the canonical-bytes projection.
    #[must_use]
    pub fn cmp_value(&self, value: &Value) -> Ordering {
        let json = &value.0;
        let (own, other) = (key_rank(self), value_rank(json));
        if own != other {
            return own.cmp(&other);
        }
        match (self, json) {
            (Self::Null, _) => Ordering::Equal,
            (Self::Bool(left), Json::Bool(right)) => left.cmp(right),
            (Self::I64(left), Json::Number(right)) => right
                .as_i64()
                .map_or(Ordering::Equal, |right| left.cmp(&right)),
            (Self::U64(left), Json::Number(right)) => right
                .as_u64()
                .map_or(Ordering::Equal, |right| left.cmp(&right)),
            (Self::F64(left), Json::Number(right)) => {
                left.cmp(&Bits::new(right.as_f64().unwrap_or(f64::NAN)))
            }
            (Self::Str(left), Json::String(right)) => left.as_ref().cmp(right.as_str()),
            _ => self.cmp(&Self::from(value)),
        }
    }

    /// Whether this key equals the key `value` would project to.
    #[must_use]
    pub fn eq_value(&self, value: &Value) -> bool {
        self.cmp_value(value) == Ordering::Equal
    }
}

const SEED: u64 = 0x517c_c1b7_2722_0a95;

fn mix(state: u64, word: u64) -> u64 {
    (state.rotate_left(5) ^ word).wrapping_mul(SEED)
}

fn mix_bytes(mut state: u64, bytes: &[u8]) -> u64 {
    let (chunks, rest) = bytes.as_chunks::<8>();
    for chunk in chunks {
        state = mix(state, u64::from_le_bytes(*chunk));
    }
    if !rest.is_empty() {
        let mut word = [0u8; 8];
        word[..rest.len()].copy_from_slice(rest);
        state = mix(state, u64::from_le_bytes(word));
    }
    mix(state, bytes.len() as u64)
}

fn finish(state: u64) -> u64 {
    let state = (state ^ (state >> 33)).wrapping_mul(0xff51_afd7_ed55_8ccd);
    state ^ (state >> 33)
}

fn hash_key(key: &Key) -> u64 {
    let tag = u64::from(key_rank(key));
    let state = match key {
        Key::Null => mix(0, tag),
        Key::Bool(value) => mix(mix(0, tag), u64::from(*value)),
        Key::I64(value) => mix(mix(0, tag), *value as u64),
        Key::U64(value) => mix(mix(0, tag), *value),
        Key::F64(bits) => mix(mix(0, tag), bits.get().to_bits()),
        Key::Str(value) => mix_bytes(mix(0, tag), value.as_bytes()),
        Key::Bytes(value) => mix_bytes(mix(0, tag), value),
    };
    finish(state)
}

fn hash_value(value: &Value) -> u64 {
    let tag = u64::from(value_rank(&value.0));
    let state = match &value.0 {
        Json::Null => mix(0, tag),
        Json::Bool(value) => mix(mix(0, tag), u64::from(*value)),
        Json::Number(number) => {
            let word = number.as_i64().map_or_else(
                || {
                    number.as_u64().unwrap_or_else(|| {
                        Bits::new(number.as_f64().unwrap_or(f64::NAN))
                            .get()
                            .to_bits()
                    })
                },
                |value| value as u64,
            );
            mix(mix(0, tag), word)
        }
        Json::String(value) => mix_bytes(mix(0, tag), value.as_bytes()),
        Json::Array(_) | Json::Object(_) => return hash_key(&Key::from(value)),
    };
    finish(state)
}

/// Hash a tuple of optional values so equal key tuples hash equally.
///
/// An absent value hashes like `null`, matching how grouping projects a
/// missing field to [`Key::Null`]. Scalars are hashed in place.
#[must_use]
pub fn hash_values<'a>(values: impl IntoIterator<Item = Option<&'a Value>>) -> u64 {
    let null = finish(mix(0, 0));
    let state = values
        .into_iter()
        .fold(0, |state, value| mix(state, value.map_or(null, hash_value)));
    finish(state)
}

/// Largest set probed by scanning instead of hashing.
const LINEAR_LIMIT: usize = 8;
const EMPTY: u32 = u32::MAX;

/// An immutable set of [`Key`]s with allocation-free membership probes.
///
/// Small sets are scanned; larger ones use an open-addressing table over a
/// dense key vector. Probing with a borrowed [`Value`] never builds a key for
/// scalar values.
#[derive(Clone, Default)]
pub struct KeySet {
    keys: Vec<Key>,
    hashes: Vec<u64>,
    table: Box<[u32]>,
}

impl KeySet {
    /// Number of distinct keys.
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether the set has no keys.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Iterate the keys in insertion order.
    pub fn iter(&self) -> std::slice::Iter<'_, Key> {
        self.keys.iter()
    }

    /// Whether the set contains `key`.
    #[must_use]
    pub fn contains(&self, key: &Key) -> bool {
        if self.table.is_empty() {
            return self.keys.contains(key);
        }
        let hash = hash_key(key);
        self.probe(hash, |candidate| candidate == key)
    }

    /// Whether the set contains the key `value` projects to.
    #[must_use]
    pub fn contains_value(&self, value: &Value) -> bool {
        if self.table.is_empty() {
            return self.keys.iter().any(|key| key.eq_value(value));
        }
        let hash = hash_value(value);
        self.probe(hash, |candidate| candidate.eq_value(value))
    }

    fn probe(&self, hash: u64, equals: impl Fn(&Key) -> bool) -> bool {
        let mask = self.table.len() - 1;
        let mut slot = hash as usize & mask;
        loop {
            let index = self.table[slot];
            if index == EMPTY {
                return false;
            }
            let index = index as usize;
            if self.hashes[index] == hash && equals(&self.keys[index]) {
                return true;
            }
            slot = (slot + 1) & mask;
        }
    }

    fn insert(&mut self, key: Key) {
        if self.contains(&key) {
            return;
        }
        let hash = hash_key(&key);
        self.keys.push(key);
        self.hashes.push(hash);
    }

    fn index(&mut self) {
        if self.keys.len() <= LINEAR_LIMIT {
            self.table = Box::default();
            return;
        }
        let size = (self.keys.len() * 2).next_power_of_two();
        let mask = size - 1;
        let mut table = vec![EMPTY; size].into_boxed_slice();
        for (index, hash) in self.hashes.iter().enumerate() {
            let mut slot = *hash as usize & mask;
            while table[slot] != EMPTY {
                slot = (slot + 1) & mask;
            }
            table[slot] = u32::try_from(index).unwrap_or(EMPTY - 1);
        }
        self.table = table;
    }
}

impl FromIterator<Key> for KeySet {
    fn from_iter<I: IntoIterator<Item = Key>>(iter: I) -> Self {
        let mut set = Self::default();
        for key in iter {
            set.insert(key);
        }
        set.index();
        set
    }
}

impl<'a> IntoIterator for &'a KeySet {
    type Item = &'a Key;
    type IntoIter = std::slice::Iter<'a, Key>;
    fn into_iter(self) -> Self::IntoIter {
        self.keys.iter()
    }
}

impl PartialEq for KeySet {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.keys.iter().all(|key| other.contains(key))
    }
}

impl Eq for KeySet {}

impl std::fmt::Debug for KeySet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set().entries(&self.keys).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<Value> {
        vec![
            Value::null(),
            Value::bool(false),
            Value::bool(true),
            Value::integer(-3),
            Value::integer(7),
            Value::new(serde_json::Value::from(u64::MAX)),
            Value::float(1.5),
            Value::float(-0.0),
            Value::float(f64::NAN),
            Value::string(""),
            Value::string("acme"),
            Value::string("acme corp"),
            Value::from(vec![Value::integer(1)]),
            Value::new(serde_json::json!({"a": 1})),
        ]
    }

    #[test]
    fn comparison_agrees_with_the_projected_key() {
        for key_source in samples() {
            let key = Key::from(&key_source);
            for value in samples() {
                assert_eq!(
                    key.cmp_value(&value),
                    key.cmp(&Key::from(&value)),
                    "{key:?} vs {value:?}"
                );
                assert_eq!(key.eq_value(&value), key == Key::from(&value));
            }
        }
    }

    #[test]
    fn membership_agrees_with_the_projected_key() {
        for width in [0, 3, 8, 9, 40] {
            let mut keys: Vec<Key> = samples().iter().map(Key::from).collect();
            keys.extend((0..width).map(|index| Key::string(format!("k{index}"))));
            let set: KeySet = keys.iter().cloned().collect();
            for value in samples()
                .into_iter()
                .chain((0..width + 2).map(|index| Value::string(format!("k{index}"))))
            {
                let key = Key::from(&value);
                assert_eq!(set.contains(&key), keys.contains(&key), "{value:?}");
                assert_eq!(set.contains_value(&value), keys.contains(&key), "{value:?}");
            }
        }
    }

    #[test]
    fn value_tuples_hash_like_their_projected_keys() {
        let hash =
            |keys: &[Key]| finish(keys.iter().fold(0, |state, key| mix(state, hash_key(key))));
        let values = [Value::string("acme"), Value::integer(4), Value::null()];
        let tuple = hash_values([Some(&values[0]), Some(&values[1]), None]);
        let keys = [Key::string("acme"), Key::I64(4), Key::Null];
        assert_eq!(tuple, hash(&keys));
        assert_eq!(
            hash_values([Some(&values[2])]),
            hash_values([None]),
            "an absent field groups with null"
        );
    }

    #[test]
    fn duplicates_collapse_and_equality_ignores_order() {
        let left: KeySet = [Key::I64(1), Key::I64(2), Key::I64(1)]
            .into_iter()
            .collect();
        let right: KeySet = [Key::I64(2), Key::I64(1)].into_iter().collect();
        assert_eq!(left.len(), 2);
        assert_eq!(left, right);
        assert!(KeySet::default().is_empty());
        assert!(!KeySet::default().contains(&Key::Null));
    }
}
