use super::validation::value_fits;
use super::*;

/// A spec that could not be compiled, with everything wrong with it.
///
/// Unlike a single [`Error`], this carries the whole diagnostic set, because the
/// useful thing to tell someone who just wrote a spec is every mistake in it,
/// not the first one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecError {
    pub(super) diagnostics: Vec<Diagnostic>,
}

impl SpecError {
    /// Build an error from a set of diagnostics.
    #[must_use]
    pub fn new(diagnostics: Vec<Diagnostic>) -> Self {
        Self { diagnostics }
    }

    /// Build an error carrying a single diagnostic.
    #[must_use]
    pub fn single(diagnostic: Diagnostic) -> Self {
        Self {
            diagnostics: vec![diagnostic],
        }
    }

    /// Every diagnostic, errors and warnings alike.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The error-level diagnostics.
    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|item| item.is_error())
    }

    /// The warning-level diagnostics.
    pub fn warnings(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|item| item.is_warning())
    }

    /// Whether there is nothing to report.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    /// Consume the error, yielding its diagnostics.
    #[must_use]
    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    /// Convert into the runtime's error type, preserving the diagnostics.
    #[must_use]
    pub fn into_error(self) -> Error {
        Error::validation(self.to_string()).with_diagnostics(self.diagnostics)
    }
}

impl std::fmt::Display for SpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let errors = self.errors().count();
        write!(
            f,
            "spec compilation failed with {errors} error{}",
            if errors == 1 { "" } else { "s" }
        )?;
        for diagnostic in self.errors().take(3) {
            write!(f, "; {}", diagnostic.message)?;
        }
        let remaining = errors.saturating_sub(3);
        if remaining > 0 {
            write!(f, "; and {remaining} more")?;
        }
        Ok(())
    }
}

impl std::error::Error for SpecError {}

impl From<SpecError> for Error {
    fn from(value: SpecError) -> Self {
        value.into_error()
    }
}

/// Where a property on a compiled object type came from.
///
/// Kept so a diagnostic can say *which trait* contributed a colliding property,
/// which is the one thing an author cannot work out from the flattened form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyOrigin {
    /// The object type declares the property itself.
    Declared,
    /// A trait contributed the property.
    Trait(ApiName),
}

impl std::fmt::Display for PropertyOrigin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Declared => f.write_str("declared"),
            Self::Trait(name) => write!(f, "trait {name}"),
        }
    }
}

/// A property of a compiled object type.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledProperty {
    /// Dense id, local to the owning object type.
    pub(super) id: PropertyId,
    /// API name.
    pub(super) api_name: ApiName,
    /// Display name.
    pub(super) display: Option<String>,
    /// Description.
    pub(super) description: Option<String>,
    /// Declared data type.
    pub(super) data_type: DataType,
    /// Whether `null` is an acceptable value, normalized from the IR's tri-state.
    pub(super) nullable: bool,
    /// Value supplied when a create omits this property.
    pub(super) default: Option<Value>,
    /// The closed set of accepted values, when one is declared.
    pub(super) allowed_values: Option<Arc<[Value]>>,
    /// Where the property came from.
    pub(super) origin: PropertyOrigin,
    /// Tags.
    pub(super) tags: Vec<String>,
}

impl CompiledProperty {
    /// Dense id, local to the owning object type.
    #[must_use]
    pub fn id(&self) -> PropertyId {
        self.id
    }

    /// API name.
    #[must_use]
    pub fn api_name(&self) -> &ApiName {
        &self.api_name
    }

    /// Display name.
    #[must_use]
    pub fn display(&self) -> Option<&str> {
        self.display.as_deref()
    }

    /// Description.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Declared data type.
    #[must_use]
    pub fn data_type(&self) -> DataType {
        self.data_type
    }

    /// Whether `null` is an acceptable value.
    #[must_use]
    pub fn is_nullable(&self) -> bool {
        self.nullable
    }

    /// Whether a create must supply this property.
    #[must_use]
    pub fn is_required(&self) -> bool {
        !self.nullable
    }

    /// The default applied when a create omits this property.
    #[must_use]
    pub fn default_value(&self) -> Option<&Value> {
        self.default.as_ref()
    }

    /// The closed set of accepted values, when one is declared.
    #[must_use]
    pub fn allowed_values(&self) -> Option<&[Value]> {
        self.allowed_values.as_deref()
    }

    /// Where the property came from.
    #[must_use]
    pub fn origin(&self) -> &PropertyOrigin {
        &self.origin
    }

    /// Tags.
    #[must_use]
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// Whether the property accepts the given non-null value.
    #[must_use]
    pub fn allows(&self, value: &Value) -> bool {
        match &self.allowed_values {
            Some(allowed) => allowed.iter().any(|candidate| candidate == value),
            None => true,
        }
    }

    /// Whether the value's shape matches the declared data type.
    ///
    /// This is a shape check, not a coercion: `"5"` does not fit an integer
    /// property. Being strict here is what keeps a JSON-shaped backend and a
    /// columnar one from disagreeing about what a record contains.
    #[must_use]
    pub fn accepts_type(&self, value: &Value) -> bool {
        value_fits(value, self.data_type)
    }
}

/// An object type with its traits flattened and its references resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledObjectType {
    pub(super) id: ObjectTypeId,
    pub(super) api_name: ApiName,
    pub(super) display: Option<String>,
    pub(super) description: Option<String>,
    pub(super) primary_key: PropertyId,
    pub(super) properties: Vec<CompiledProperty>,
    pub(super) property_index: HashMap<ApiName, PropertyId>,
    pub(super) traits: Vec<ApiName>,
    pub(super) tags: Vec<String>,
    pub(super) deprecated_at: Option<String>,
}

impl CompiledObjectType {
    /// Dense id within the owning ontology.
    #[must_use]
    pub fn id(&self) -> ObjectTypeId {
        self.id
    }

    /// API name.
    #[must_use]
    pub fn api_name(&self) -> &ApiName {
        &self.api_name
    }

    /// Display name.
    #[must_use]
    pub fn display(&self) -> Option<&str> {
        self.display.as_deref()
    }

    /// Description.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Tags.
    #[must_use]
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// When the object type was deprecated, if it was.
    #[must_use]
    pub fn deprecated_at(&self) -> Option<&str> {
        self.deprecated_at.as_deref()
    }

    /// The traits this object type implements.
    #[must_use]
    pub fn traits(&self) -> &[ApiName] {
        &self.traits
    }

    /// Every property, declared first and trait-contributed after.
    #[must_use]
    pub fn properties(&self) -> &[CompiledProperty] {
        &self.properties
    }

    /// Look a property up by name.
    #[must_use]
    pub fn property(&self, name: &str) -> Option<&CompiledProperty> {
        self.property_index
            .get(name)
            .map(|id| &self.properties[id.index()])
    }

    /// Look a property up by id.
    #[must_use]
    pub fn property_by_id(&self, id: PropertyId) -> Option<&CompiledProperty> {
        self.properties.get(id.index())
    }

    /// The property that identifies a record.
    #[must_use]
    pub fn primary_key(&self) -> &CompiledProperty {
        // The compiler guarantees the id resolves, so this cannot fail.
        &self.properties[self.primary_key.index()]
    }

    /// The id of the primary key property.
    #[must_use]
    pub fn primary_key_id(&self) -> PropertyId {
        self.primary_key
    }

    /// Apply declared defaults to a value map in place.
    ///
    /// Only fills gaps: a value the caller supplied always wins, including an
    /// explicit `null`.
    pub fn apply_defaults(&self, values: &mut BTreeMap<ApiName, Value>) {
        for property in &self.properties {
            if let Some(default) = &property.default
                && !values.contains_key(&property.api_name)
            {
                values.insert(property.api_name.clone(), default.clone());
            }
        }
    }
}

impl std::fmt::Display for CompiledObjectType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.api_name)
    }
}
