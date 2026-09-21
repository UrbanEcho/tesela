//! Strongly-typed identifiers for Tesela.

use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::fmt;
use std::ops::Deref;
use std::str::FromStr;
use std::sync::Arc;

/// A validated API name: lowercase ASCII letters, digits, and underscores,
/// starting with a letter.
///
/// # Examples
///
/// ```
/// use tesela_core::ApiName;
/// let name = ApiName::new("customer_order")?;
/// assert_eq!(name.as_ref(), "customer_order");
/// # Ok::<(), tesela_core::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ApiName(Arc<str>);

/// A typed source for stable Tesela API names.
///
/// Platform code should implement this trait for enums that name backends,
/// object types, fields, tools, and other durable contracts. This keeps string
/// literals at the boundary of the enum implementation instead of spreading
/// magic strings across ontology definitions.
pub trait ApiNameSource {
    /// Return the stable API name.
    fn api_name(&self) -> &'static str;

    /// Convert the stable API name into a validated Tesela identifier.
    fn to_api_name(&self) -> ApiName {
        ApiName::from(self.api_name())
    }
}

impl ApiNameSource for &'static str {
    fn api_name(&self) -> &'static str {
        self
    }
}

impl ApiName {
    /// Description of the valid API name grammar, for error messages.
    pub const PATTERN: &'static str = "[a-z][a-z0-9_]*";

    /// Create a new `ApiName`, validating the input.
    ///
    /// Returns an error if the string does not match the API name pattern.
    pub fn new<S: AsRef<str>>(s: S) -> Result<Self, crate::Error> {
        let s = s.as_ref();
        if !Self::is_valid(s) {
            return Err(crate::Error::validation(format!(
                "api_name '{s}' does not match pattern {pattern}",
                pattern = Self::PATTERN
            )));
        }
        Ok(Self(Arc::from(s)))
    }

    /// Whether a string is a valid API name.
    ///
    /// This is a byte scan rather than a regular expression: it is called on
    /// every spec load and every name that crosses a language boundary, and it
    /// needs no allocation.
    #[must_use]
    pub fn is_valid(s: &str) -> bool {
        let bytes = s.as_bytes();
        match bytes.split_first() {
            Some((first, rest)) => {
                first.is_ascii_lowercase()
                    && rest.iter().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_'
                    })
            }
            None => false,
        }
    }

    /// Create an `ApiName` without validation.
    ///
    /// The caller must ensure the string matches the API name pattern. Prefer
    /// `ApiName::new` or `parse` in production code. Panics in debug builds if
    /// the input is invalid, so mistakes surface in tests.
    pub fn new_unchecked<S: AsRef<str>>(s: S) -> Self {
        let s = s.as_ref();
        debug_assert!(
            Self::is_valid(s),
            "'{s}' is not a valid ApiName (must match {})",
            Self::PATTERN
        );
        Self(Arc::from(s))
    }

    /// The name as a string slice.
    ///
    /// Borrowing is allocation-free, so this is the accessor to prefer over
    /// `as_ref()` when the target type is inferred ambiguously.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for ApiName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Allows `BTreeMap<ApiName, V>::get("field")` and `HashMap<ApiName, V>::get`
/// without constructing an `ApiName` — and therefore without an `Arc`
/// allocation — on every lookup.
impl Borrow<str> for ApiName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for ApiName {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for ApiName {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<ApiName> for str {
    fn eq(&self, other: &ApiName) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<ApiName> for &str {
    fn eq(&self, other: &ApiName) -> bool {
        *self == other.as_str()
    }
}

impl Deref for ApiName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for ApiName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for ApiName {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// Create an `ApiName` from a `&'static str` without runtime validation.
///
/// Use for string literals known at compile time to be valid API names.
/// Analogous to `http::Method::from_static`. Panics in debug builds if the
/// string is not a valid API name, so mistakes surface in tests rather than
/// production.
impl From<&'static str> for ApiName {
    fn from(s: &'static str) -> Self {
        debug_assert!(
            Self::new(s).is_ok(),
            "'{s}' is not a valid ApiName (must match {})",
            Self::PATTERN
        );
        Self::new_unchecked(s)
    }
}

impl From<ApiName> for String {
    fn from(value: ApiName) -> Self {
        value.0.to_string()
    }
}

impl From<&ApiName> for String {
    fn from(value: &ApiName) -> Self {
        value.0.to_string()
    }
}

impl Serialize for ApiName {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ApiName {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

/// A version string for the Tesela spec.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(Arc<str>);

impl Version {
    /// Create a new version.
    pub fn new<S: AsRef<str>>(s: S) -> Self {
        Self(Arc::from(s.as_ref()))
    }

    /// The version as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Version {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for Version {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for Version {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Version {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(s))
    }
}

impl Serialize for Version {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self::new(s))
    }
}
