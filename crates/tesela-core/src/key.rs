//! Canonical scalar keys derived from [`Value`]s.
//!
//! A [`Key`] is the hashable, totally-ordered projection of a [`Value`] that
//! Tesela uses wherever a value must serve as an identity or a map key:
//! record primary keys, `IN` filter sets, aggregation group keys, and set
//! operations over result pages.
//!
//! The point of the type is to make those paths allocation-free and
//! *consistent*. [`Value`] can do none of this well: its [`Ord`] allocates two
//! `Vec`s and clones every operand when it compares objects, and it disagrees
//! with its own [`Eq`] on numbers (`Ord` calls `1` and `1.0` equal, `Eq` calls
//! them distinct), so a `BTreeMap` keyed by `Value` can behave incoherently.
//! [`Key`] fixes both: ordering, equality and hashing all agree, and scalar
//! keys carry no heap allocation beyond a shared `Arc` for strings.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::value::Value;

/// A canonical, hashable, totally-ordered projection of a [`Value`].
///
/// Scalar JSON values map to their natural variants. Arrays and objects, which
/// have no meaningful scalar identity, are canonicalised to their compact JSON
/// encoding in [`Key::Bytes`] so that every `Value` has a `Key` and equality is
/// still exact.
///
/// Ordering is total and consistent with equality. The variant order is a
/// deliberate, stable contract (`Null < Bool < Number < String < Bytes`) so that
/// ordering group keys produces deterministic output.
/// ## Serialization
///
/// A `Key` serializes as the JSON value it was built from — `Key::string("a")`
/// becomes `"a"`, `Key::I64(1)` becomes `1` — rather than as a tagged enum.
/// This keeps a record's primary key flat on the wire and over the Python
/// boundary, matching the `Value` it replaced. [`Serialize`] and
/// [`Deserialize`] are hand-written because `Arc<str>` and `Arc<[u8]>` have no
/// serde impls; the round trip is exact for every variant.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    /// JSON `null`.
    Null,
    /// A boolean.
    Bool(bool),
    /// A signed integer.
    I64(i64),
    /// An unsigned integer that does not fit in `i64`.
    U64(u64),
    /// A float, stored by bit pattern with `-0.0` canonicalised to `0.0` and all
    /// NaNs canonicalised to a single quiet NaN.
    F64(Bits),
    /// A string.
    Str(Arc<str>),
    /// The canonical JSON encoding of a value with no scalar identity.
    Bytes(Arc<[u8]>),
}

/// An `f64` that is `Eq`, `Hash` and `Ord`.
///
/// NaN is canonicalised so that `Eq` stays reflexive and hashing is stable;
/// `-0.0` is normalised to `0.0` so that `Eq` agrees with the numeric ordering.
#[derive(Debug, Clone, Copy)]
pub struct Bits(u64);

impl Bits {
    /// Canonicalise a float into its comparable bit pattern.
    #[must_use]
    pub fn new(value: f64) -> Self {
        if value.is_nan() {
            // One quiet NaN for every NaN payload.
            return Self(f64::NAN.to_bits());
        }
        if value == 0.0 {
            // Collapse -0.0 onto 0.0.
            return Self(0f64.to_bits());
        }
        Self(value.to_bits())
    }

    /// The original float.
    #[must_use]
    pub fn get(self) -> f64 {
        f64::from_bits(self.0)
    }
}

impl PartialEq for Bits {
    fn eq(&self, other: &Self) -> bool {
        // Bit patterns are already canonical, so bit equality is value equality.
        self.0 == other.0
    }
}

impl Eq for Bits {}

impl std::hash::Hash for Bits {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl PartialOrd for Bits {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Bits {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.get()
            .partial_cmp(&other.get())
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

impl From<f64> for Bits {
    fn from(value: f64) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for Bits {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get())
    }
}

impl Key {
    /// The JSON type name of this key, for diagnostics.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::I64(_) | Self::U64(_) | Self::F64(_) => "number",
            Self::Str(_) => "string",
            Self::Bytes(_) => "struct",
        }
    }

    /// Whether this key is the null key.
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// The key as a string slice, when it is one.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(value) => Some(value),
            _ => None,
        }
    }

    /// The key as a float, widening integers.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::I64(value) => Some(*value as f64),
            Self::U64(value) => Some(*value as f64),
            Self::F64(bits) => Some(bits.get()),
            _ => None,
        }
    }

    /// The key as a signed integer, when it is one.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::I64(value) => Some(*value),
            Self::U64(value) => i64::try_from(*value).ok(),
            _ => None,
        }
    }

    /// Build a key from an owned string.
    #[must_use]
    pub fn string<S: AsRef<str>>(value: S) -> Self {
        Self::Str(Arc::from(value.as_ref()))
    }

    /// Build a key from a float.
    #[must_use]
    pub fn float(value: f64) -> Self {
        Self::F64(Bits::new(value))
    }

    /// Recover the [`Value`] this key was projected from.
    ///
    /// This is the inverse of [`Key::from`] for every scalar variant. A
    /// [`Key::Bytes`] round-trips by re-parsing the JSON it encodes, so the
    /// result always compares equal to the original value under [`Value`]'s
    /// equality — though not necessarily bit-for-bit for a float, since
    /// canonicalisation collapses `-0.0` and NaN payloads.
    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Null => Value::null(),
            Self::Bool(value) => Value::bool(*value),
            Self::I64(value) => Value::integer(*value),
            Self::U64(value) => Value::new(serde_json::Value::from(*value)),
            Self::F64(bits) => Value::float(bits.get()),
            Self::Str(value) => Value::string(value.as_ref()),
            Self::Bytes(value) => serde_json::from_slice(value)
                .map(Value::new)
                .unwrap_or_else(|_| Value::string(String::from_utf8_lossy(value).into_owned())),
        }
    }
}

impl From<&Value> for Key {
    fn from(value: &Value) -> Self {
        use serde_json::Value as Json;
        match &value.0 {
            Json::Null => Self::Null,
            Json::Bool(value) => Self::Bool(*value),
            Json::Number(number) => match number.as_i64() {
                Some(value) => Self::I64(value),
                None => match number.as_u64() {
                    Some(value) => Self::U64(value),
                    None => Self::F64(Bits::new(number.as_f64().unwrap_or(f64::NAN))),
                },
            },
            Json::String(value) => Self::Str(Arc::from(value.as_str())),
            // Arrays and objects have no scalar identity. `serde_json` writes
            // object keys in sorted order, so the encoding is canonical for a
            // given map regardless of insertion order.
            other => Self::Bytes(Arc::from(
                serde_json::to_string(other)
                    .unwrap_or_default()
                    .into_bytes()
                    .into_boxed_slice(),
            )),
        }
    }
}

impl From<Value> for Key {
    fn from(value: Value) -> Self {
        Self::from(&value)
    }
}

/// Why a [`Value`] could not be used as an identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    /// The value is `null`, which is never a valid identity.
    NullKey,
    /// The value is an array or object, which has no scalar identity.
    NotScalar,
    /// The value is a float, which is not a stable identity.
    FloatKey,
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NullKey => "a null value cannot be a primary key",
            Self::NotScalar => "an array or object cannot be a primary key",
            Self::FloatKey => "a float cannot be a primary key",
        })
    }
}

impl std::error::Error for KeyError {}

impl Key {
    /// Build a strict *identity* key, rejecting values that are not stable
    /// identities.
    ///
    /// Use this for primary keys, where a float or a nested structure is a
    /// modelling error rather than something to encode. This is deliberately an
    /// inherent method rather than a `TryFrom` impl: the blanket
    /// `TryFrom`-from-`Into` impl would make it unreachable alongside
    /// `From<&Value>`.
    pub fn identity(value: &Value) -> Result<Self, KeyError> {
        match Self::from(value) {
            Self::Null => Err(KeyError::NullKey),
            Self::F64(_) => Err(KeyError::FloatKey),
            Self::Bytes(_) => Err(KeyError::NotScalar),
            key => Ok(key),
        }
    }
}

impl Serialize for Key {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::I64(value) => serializer.serialize_i64(*value),
            Self::U64(value) => serializer.serialize_u64(*value),
            Self::F64(bits) => serializer.serialize_f64(bits.get()),
            Self::Str(value) => serializer.serialize_str(value),
            // `Bytes` holds the canonical JSON this key was built from, so it
            // round-trips by re-emitting that JSON.
            Self::Bytes(value) => {
                let decoded: serde_json::Value =
                    serde_json::from_slice(value).map_err(serde::ser::Error::custom)?;
                decoded.serialize(serializer)
            }
        }
    }
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Deserialization goes through `serde_json::Value`, so `Key` is only
        // deserializable from a self-describing format. Every Tesela boundary
        // is JSON.
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(Self::from(&Value::new(value)))
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("null"),
            Self::Bool(value) => write!(f, "{value}"),
            Self::I64(value) => write!(f, "{value}"),
            Self::U64(value) => write!(f, "{value}"),
            Self::F64(bits) => write!(f, "{}", bits.get()),
            Self::Str(value) => f.write_str(value),
            Self::Bytes(value) => f.write_str(&String::from_utf8_lossy(value)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    fn hash_of<T: Hash>(value: &T) -> u64 {
        let mut hasher = DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn scalars_project_to_their_natural_variants() {
        assert_eq!(Key::from(&Value::null()), Key::Null);
        assert_eq!(Key::from(&Value::bool(true)), Key::Bool(true));
        assert_eq!(Key::from(&Value::integer(-7)), Key::I64(-7));
        assert_eq!(Key::from(&Value::string("a")), Key::string("a"));
        assert_eq!(Key::from(&Value::float(1.5)), Key::float(1.5));
    }

    #[test]
    fn ordering_is_consistent_with_equality_for_numbers() {
        // `Value`'s own `Ord` treats `1` and `1.0` as equal while its `Eq` does
        // not. `Key` must not inherit that incoherence.
        let integer = Key::from(&Value::integer(1));
        let float = Key::from(&Value::float(1.0));
        assert_ne!(integer, float);
        assert_ne!(integer.cmp(&float), std::cmp::Ordering::Equal);
    }

    #[test]
    fn negative_zero_and_zero_are_the_same_key() {
        let zero = Key::float(0.0);
        let negative = Key::float(-0.0);
        assert_eq!(zero, negative);
        assert_eq!(hash_of(&zero), hash_of(&negative));
    }

    #[test]
    fn every_nan_is_the_same_key() {
        let plain = Key::float(f64::NAN);
        let payload = Key::float(f64::from_bits(0x7ff8_0000_dead_beef));
        assert_eq!(plain, payload);
        assert_eq!(hash_of(&plain), hash_of(&payload));
    }

    #[test]
    fn equality_and_hashing_agree() {
        let left = Key::from(&Value::string("customer"));
        let right = Key::from(&Value::string("customer"));
        assert_eq!(left, right);
        assert_eq!(hash_of(&left), hash_of(&right));
    }

    #[test]
    fn containers_fall_back_to_canonical_bytes() {
        let array = Key::from(&Value::from(vec![Value::integer(1), Value::integer(2)]));
        assert!(matches!(array, Key::Bytes(_)));
        assert_eq!(array.to_string(), "[1,2]");
    }

    #[test]
    fn keys_round_trip_back_to_values() {
        for value in [
            Value::null(),
            Value::bool(true),
            Value::integer(-7),
            Value::float(1.5),
            Value::string("acme"),
            Value::from(vec![Value::integer(1), Value::integer(2)]),
        ] {
            let key = Key::from(&value);
            assert_eq!(
                key.to_value(),
                value,
                "{key:?} did not round-trip back to {value:?}"
            );
        }
    }

    #[test]
    fn strict_conversion_rejects_unstable_identities() {
        assert_eq!(Key::identity(&Value::null()), Err(KeyError::NullKey));
        assert_eq!(Key::identity(&Value::float(1.0)), Err(KeyError::FloatKey));
        assert_eq!(
            Key::identity(&Value::from(vec![Value::integer(1)])),
            Err(KeyError::NotScalar)
        );
        assert_eq!(
            Key::identity(&Value::string("id-1")).expect("a string is a stable identity"),
            Key::string("id-1")
        );
    }

    #[test]
    fn serialization_is_flat_and_round_trips() {
        let cases = vec![
            (Key::Null, "null"),
            (Key::Bool(true), "true"),
            (Key::I64(-7), "-7"),
            (Key::U64(u64::MAX), "18446744073709551615"),
            (Key::float(1.5), "1.5"),
            (Key::string("customer_1"), "\"customer_1\""),
            (
                Key::from(&Value::from(vec![Value::integer(1), Value::integer(2)])),
                "[1,2]",
            ),
        ];
        for (key, expected) in cases {
            let json = serde_json::to_string(&key).expect("key should serialize");
            assert_eq!(json, expected, "unexpected wire form for {key:?}");
            let parsed: Key = serde_json::from_str(&json).expect("key should deserialize");
            assert_eq!(parsed, key, "round trip changed {key:?}");
        }
    }

    #[test]
    fn nested_keys_round_trip_through_a_record_shape() {
        // The primary key appears nested inside a larger structure on the wire.
        let original = Key::string("cust_0001");
        let json = serde_json::to_string(&serde_json::json!({ "id": original }))
            .expect("record should serialize");
        assert_eq!(json, r#"{"id":"cust_0001"}"#);
    }

    #[test]
    fn variant_order_is_stable() {
        let mut keys = vec![
            Key::Bytes(Arc::from(&b"{}"[..])),
            Key::string("a"),
            Key::float(1.0),
            Key::Bool(false),
            Key::Null,
        ];
        keys.sort();
        assert_eq!(
            keys,
            vec![
                Key::Null,
                Key::Bool(false),
                Key::float(1.0),
                Key::string("a"),
                Key::Bytes(Arc::from(&b"{}"[..])),
            ]
        );
    }
}
