//! Value wrapper for record data.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::fmt;
use std::ops::Deref;

/// A thin newtype around `serde_json::Value` that provides [`Ord`] support
/// so that values can be used as keys in `BTreeMap` when needed.
///
/// The ordering is best-effort: strings < numbers < booleans < null < arrays < objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct Value(pub serde_json::Value);

impl Value {
    /// Create a `Value` from a `serde_json::Value`.
    pub fn new(v: serde_json::Value) -> Self {
        Self(v)
    }

    /// Create a null value.
    pub fn null() -> Self {
        Self(serde_json::Value::Null)
    }

    /// Create a string value.
    pub fn string<S: Into<String>>(s: S) -> Self {
        Self(serde_json::Value::String(s.into()))
    }

    /// Create a boolean value.
    pub fn bool(b: bool) -> Self {
        Self(serde_json::Value::Bool(b))
    }

    /// Create an integer value.
    pub fn integer(i: i64) -> Self {
        Self(serde_json::Value::from(i))
    }

    /// Create a float value.
    pub fn float(f: f64) -> Self {
        Self(serde_json::Value::from(f))
    }

    /// Returns `true` if this value is null.
    pub fn is_null(&self) -> bool {
        self.0.is_null()
    }

    /// Returns the value as a string, if it is one.
    pub fn as_str(&self) -> Option<&str> {
        self.0.as_str()
    }

    /// Returns the value as an integer, if it is one.
    pub fn as_i64(&self) -> Option<i64> {
        self.0.as_i64()
    }

    /// Returns the value as a float, if it is one.
    pub fn as_f64(&self) -> Option<f64> {
        self.0.as_f64()
    }

    /// Returns the value as a bool, if it is one.
    pub fn as_bool(&self) -> Option<bool> {
        self.0.as_bool()
    }

    /// Returns the value as an array, if it is one.
    pub fn as_array(&self) -> Option<&Vec<serde_json::Value>> {
        self.0.as_array()
    }

    /// Returns the value as an object, if it is one.
    pub fn as_object(&self) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.0.as_object()
    }
}

impl Deref for Value {
    type Target = serde_json::Value;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<serde_json::Value> for Value {
    fn from(v: serde_json::Value) -> Self {
        Self(v)
    }
}

impl From<Value> for serde_json::Value {
    fn from(v: Value) -> Self {
        v.0
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Self(serde_json::Value::String(s))
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Self(serde_json::Value::String(s.to_string()))
    }
}

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Self(serde_json::Value::from(i))
    }
}

impl From<f64> for Value {
    fn from(f: f64) -> Self {
        Self(serde_json::Value::from(f))
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Self(serde_json::Value::Bool(b))
    }
}

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Ordering rank per JSON type, defining the cross-type order
/// `null < bool < number < string < array < object`.
const fn rank(value: &serde_json::Value) -> u8 {
    match value {
        serde_json::Value::Null => 0,
        serde_json::Value::Bool(_) => 1,
        serde_json::Value::Number(_) => 2,
        serde_json::Value::String(_) => 3,
        serde_json::Value::Array(_) => 4,
        serde_json::Value::Object(_) => 5,
    }
}

/// Compare two raw JSON values.
///
/// This is a free function rather than a method so that arrays and objects can
/// recurse into it directly, without wrapping each element in a [`Value`] — the
/// wrapper is what made the previous implementation allocate and clone on every
/// comparison.
fn cmp_json(left: &serde_json::Value, right: &serde_json::Value) -> Ordering {
    use serde_json::Value as Json;
    match (left, right) {
        (Json::Null, Json::Null) => Ordering::Equal,
        (Json::Bool(left), Json::Bool(right)) => left.cmp(right),
        (Json::Number(left), Json::Number(right)) => cmp_number(left, right),
        (Json::String(left), Json::String(right)) => left.cmp(right),
        (Json::Array(left), Json::Array(right)) => left
            .len()
            .cmp(&right.len())
            .then_with(|| cmp_slices(left, right)),
        // `serde_json::Map` keeps its keys sorted, so zipping the two iterators
        // walks both in the same canonical order. No side is materialised.
        (Json::Object(left), Json::Object(right)) => left.len().cmp(&right.len()).then_with(|| {
            let mut left = left.iter();
            let mut right = right.iter();
            loop {
                match (left.next(), right.next()) {
                    (None, None) => return Ordering::Equal,
                    (None, Some(_)) => return Ordering::Less,
                    (Some(_), None) => return Ordering::Greater,
                    (Some((left_key, left_value)), Some((right_key, right_value))) => {
                        match left_key.cmp(right_key) {
                            Ordering::Equal => {}
                            other => return other,
                        }
                        match cmp_json(left_value, right_value) {
                            Ordering::Equal => {}
                            other => return other,
                        }
                    }
                }
            }
        }),
        (left, right) => rank(left).cmp(&rank(right)),
    }
}

/// Compare two JSON arrays element-wise.
fn cmp_slices(left: &[serde_json::Value], right: &[serde_json::Value]) -> Ordering {
    let mut left = left.iter();
    let mut right = right.iter();
    loop {
        match (left.next(), right.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(left), Some(right)) => match cmp_json(left, right) {
                Ordering::Equal => {}
                other => return other,
            },
        }
    }
}

/// Compare JSON numbers without collapsing distinct JSON representations.
/// This keeps `Ord` consistent with `Eq`: `1`, an unsigned integer outside the
/// signed range, and `1.0` occupy distinct variants even when their numeric
/// magnitudes can be represented by the same float.
fn cmp_number(left: &serde_json::Number, right: &serde_json::Number) -> Ordering {
    enum NumberRef {
        Signed(i64),
        Unsigned(u64),
        Float(u64),
    }
    impl NumberRef {
        const fn rank(&self) -> u8 {
            match self {
                Self::Signed(_) => 0,
                Self::Unsigned(_) => 1,
                Self::Float(_) => 2,
            }
        }
    }
    fn classify(value: &serde_json::Number) -> NumberRef {
        if let Some(value) = value.as_i64() {
            NumberRef::Signed(value)
        } else if let Some(value) = value.as_u64() {
            NumberRef::Unsigned(value)
        } else {
            NumberRef::Float(value.as_f64().unwrap_or(f64::NAN).to_bits())
        }
    }
    let left = classify(left);
    let right = classify(right);
    match (&left, &right) {
        (NumberRef::Signed(left), NumberRef::Signed(right)) => left.cmp(right),
        (NumberRef::Unsigned(left), NumberRef::Unsigned(right)) => left.cmp(right),
        (NumberRef::Float(left), NumberRef::Float(right)) => left.cmp(right),
        _ => left.rank().cmp(&right.rank()),
    }
}

impl Ord for Value {
    /// Compare two values without allocating.
    ///
    /// Ordering, equality, and hashing use the same exact JSON semantics.
    fn cmp(&self, other: &Self) -> Ordering {
        cmp_json(&self.0, &other.0)
    }
}

impl Default for Value {
    fn default() -> Self {
        Self::null()
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Vec<Value>> for Value {
    fn from(v: Vec<Value>) -> Self {
        Self(serde_json::Value::Array(
            v.into_iter().map(|x| x.0).collect(),
        ))
    }
}

impl std::hash::Hash for Value {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash based on the JSON type tag and content.
        std::mem::discriminant(&self.0).hash(state);
        match &self.0 {
            serde_json::Value::Null => {}
            serde_json::Value::Bool(b) => b.hash(state),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    i.hash(state);
                } else if let Some(f) = n.as_f64() {
                    f.to_bits().hash(state);
                }
            }
            serde_json::Value::String(s) => s.hash(state),
            serde_json::Value::Array(arr) => arr.hash(state),
            serde_json::Value::Object(map) => {
                for (k, v) in map.iter() {
                    k.hash(state);
                    v.hash(state);
                }
            }
        }
    }
}
