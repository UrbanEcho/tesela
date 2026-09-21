//! Structured diagnostics for Tesela compiler passes.

use crate::ident::ApiName;
use serde::{Deserialize, Serialize};

/// Severity level of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiagnosticLevel {
    /// Informational note.
    Info,
    /// Warning: non-fatal, should be addressed.
    Warning,
    /// Error: fatal to compilation.
    Error,
}

/// Structured diagnostic code for categorization.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    /// A name is malformed.
    InvalidName,
    /// Two entities share an API name.
    DuplicateName,
    /// A reference does not resolve (unknown object type, missing property).
    BrokenReference,
    /// An object type declaration is inconsistent.
    InvalidObjectType,
    /// A property definition is invalid.
    InvalidProperty,
    /// A trait declaration or application is invalid.
    InvalidTrait,
    /// A link type declaration is invalid.
    InvalidLink,
    /// An action declaration is invalid.
    InvalidAction,
    /// General validation failure.
    ValidationFailed,
    /// A change that breaks previously valid usage.
    BreakingChange,
    /// Custom code with a string identifier.
    Custom(String),
}

/// A single diagnostic message emitted by the compiler.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Severity level.
    pub level: DiagnosticLevel,
    /// Machine-readable code.
    pub code: DiagnosticCode,
    /// Human-readable message.
    pub message: String,
    /// Optional API name of the entity related to this diagnostic.
    pub api_name: Option<ApiName>,
}

impl Diagnostic {
    /// Create a new error-level diagnostic.
    pub fn error<M: Into<String>>(code: DiagnosticCode, message: M) -> Self {
        Self {
            level: DiagnosticLevel::Error,
            code,
            message: message.into(),
            api_name: None,
        }
    }

    /// Create a new warning-level diagnostic.
    pub fn warning<M: Into<String>>(code: DiagnosticCode, message: M) -> Self {
        Self {
            level: DiagnosticLevel::Warning,
            code,
            message: message.into(),
            api_name: None,
        }
    }

    /// Create a new info-level diagnostic.
    pub fn info<M: Into<String>>(code: DiagnosticCode, message: M) -> Self {
        Self {
            level: DiagnosticLevel::Info,
            code,
            message: message.into(),
            api_name: None,
        }
    }

    /// Attach an API name to this diagnostic.
    pub fn with_api_name(mut self, name: ApiName) -> Self {
        self.api_name = Some(name);
        self
    }

    /// Whether this diagnostic is an error.
    pub fn is_error(&self) -> bool {
        matches!(self.level, DiagnosticLevel::Error)
    }

    /// Whether this diagnostic is a warning.
    pub fn is_warning(&self) -> bool {
        matches!(self.level, DiagnosticLevel::Warning)
    }
}

/// Aggregated diagnostics from a compilation run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diagnostics {
    inner: Vec<Diagnostic>,
}

impl Diagnostics {
    /// Create empty diagnostics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a single diagnostic.
    pub fn push(&mut self, diag: Diagnostic) {
        self.inner.push(diag);
    }

    /// Extend with another collection of diagnostics.
    pub fn extend(&mut self, other: impl IntoIterator<Item = Diagnostic>) {
        self.inner.extend(other);
    }

    /// Whether any error-level diagnostic exists.
    pub fn has_errors(&self) -> bool {
        self.inner.iter().any(|d| d.is_error())
    }

    /// Whether any warning-level diagnostic exists.
    pub fn has_warnings(&self) -> bool {
        self.inner.iter().any(|d| d.is_warning())
    }

    /// Total number of diagnostics.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Whether there are no diagnostics.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Iterate over all diagnostics.
    pub fn iter(&self) -> impl Iterator<Item = &Diagnostic> {
        self.inner.iter()
    }

    /// Iterate over error-level diagnostics.
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.inner.iter().filter(|d| d.is_error())
    }

    /// Iterate over warning-level diagnostics.
    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.inner.iter().filter(|d| d.is_warning())
    }

    /// Consume and return the underlying vector.
    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.inner
    }
}

impl IntoIterator for Diagnostics {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}
