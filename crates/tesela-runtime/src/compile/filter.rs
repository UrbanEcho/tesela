//! Compilation of the filter AST into an executable predicate.

use std::collections::HashSet;
use std::sync::Arc;

use tesela_core::like::LikePlan;
use tesela_core::{ApiName, Error, FilterOp, Key, Value};
use tesela_ir::Filter;

use super::CompiledObjectType;
use super::ids::PropertyId;

/// A property reference resolved against its object type.
///
/// Both halves are needed: `id` is what a columnar backend indexes rows by, and
/// `name` is what a `Record`'s `BTreeMap` is keyed by. Carrying the name costs
/// one `Arc` clone and removes a reverse lookup from the per-record path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldRef {
    id: PropertyId,
    name: ApiName,
}

impl FieldRef {
    /// Build a field reference.
    #[must_use]
    pub fn new(id: PropertyId, name: ApiName) -> Self {
        Self { id, name }
    }

    /// The property's dense id within its object type.
    #[must_use]
    pub fn id(&self) -> PropertyId {
        self.id
    }

    /// The property's API name.
    #[must_use]
    pub fn name(&self) -> &ApiName {
        &self.name
    }

    /// Read this field out of a record's values.
    #[must_use]
    pub fn value_of<'a>(
        &self,
        values: &'a std::collections::BTreeMap<ApiName, Value>,
    ) -> Option<&'a Value> {
        values.get(&self.name)
    }
}

impl std::fmt::Display for FieldRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// An executable filter predicate.
///
/// Compilation resolves every field against the object type once, turns `In`
/// sets into hash sets, precompiles `LIKE` patterns, and shares the tree behind
/// `Arc`s so a plan clones cheaply. The tree is *closed*: everything that could
/// fail — an unknown property, a missing operand, a malformed range — has
/// already failed at compile time, so evaluation is infallible and cannot fail
/// open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompiledFilter {
    /// Every operand must match.
    And(Arc<[CompiledFilter]>),
    /// At least one operand must match.
    Or(Arc<[CompiledFilter]>),
    /// The operand must not match.
    Not(Arc<CompiledFilter>),
    /// The field is absent or `null`.
    IsNull(FieldRef),
    /// The field is present and not `null`.
    IsNotNull(FieldRef),
    /// Equality.
    Eq(FieldRef, Key),
    /// Inequality.
    Ne(FieldRef, Key),
    /// Strictly less than.
    Lt(FieldRef, Key),
    /// Less than or equal.
    Lte(FieldRef, Key),
    /// Strictly greater than.
    Gt(FieldRef, Key),
    /// Greater than or equal.
    Gte(FieldRef, Key),
    /// Membership.
    In(FieldRef, Arc<HashSet<Key>>),
    /// Non-membership.
    NotIn(FieldRef, Arc<HashSet<Key>>),
    /// Inclusive range.
    Between(FieldRef, Key, Key),
    /// Substring containment.
    Contains(FieldRef, Arc<str>),
    /// Prefix match.
    StartsWith(FieldRef, Arc<str>),
    /// SQL `LIKE`.
    Like(FieldRef, Arc<LikePlan>),
}

impl CompiledFilter {
    /// Evaluate the closed predicate without allocation or failure.
    #[must_use]
    pub fn matches(&self, values: &std::collections::BTreeMap<ApiName, Value>) -> bool {
        let value = |field: &FieldRef| field.value_of(values).map(Key::from);
        match self {
            Self::And(items) => items.iter().all(|item| item.matches(values)),
            Self::Or(items) => items.iter().any(|item| item.matches(values)),
            Self::Not(item) => !item.matches(values),
            Self::IsNull(field) => field.value_of(values).is_none_or(Value::is_null),
            Self::IsNotNull(field) => field.value_of(values).is_some_and(|item| !item.is_null()),
            Self::Eq(field, expected) => value(field).as_ref() == Some(expected),
            Self::Ne(field, expected) => value(field).as_ref() != Some(expected),
            Self::Lt(field, expected) => value(field).is_some_and(|item| item < *expected),
            Self::Lte(field, expected) => value(field).is_some_and(|item| item <= *expected),
            Self::Gt(field, expected) => value(field).is_some_and(|item| item > *expected),
            Self::Gte(field, expected) => value(field).is_some_and(|item| item >= *expected),
            Self::In(field, expected) => value(field).is_some_and(|item| expected.contains(&item)),
            Self::NotIn(field, expected) => {
                value(field).is_none_or(|item| !expected.contains(&item))
            }
            Self::Between(field, low, high) => {
                value(field).is_some_and(|item| item >= *low && item <= *high)
            }
            Self::Contains(field, expected) => field
                .value_of(values)
                .and_then(Value::as_str)
                .is_some_and(|item| item.contains(expected.as_ref())),
            Self::StartsWith(field, expected) => field
                .value_of(values)
                .and_then(Value::as_str)
                .is_some_and(|item| item.starts_with(expected.as_ref())),
            Self::Like(field, plan) => field
                .value_of(values)
                .and_then(Value::as_str)
                .is_some_and(|item| plan.matches(item)),
        }
    }

    /// Every field the filter reads, in first-seen order.
    #[must_use]
    pub fn fields(&self) -> Vec<&FieldRef> {
        let mut found = Vec::new();
        self.collect_fields(&mut found);
        found
    }

    fn collect_fields<'a>(&'a self, found: &mut Vec<&'a FieldRef>) {
        let single = |field: &'a FieldRef, found: &mut Vec<&'a FieldRef>| {
            if !found.iter().any(|seen| seen.id == field.id) {
                found.push(field);
            }
        };
        match self {
            Self::And(operands) | Self::Or(operands) => {
                for operand in operands.iter() {
                    operand.collect_fields(found);
                }
            }
            Self::Not(operand) => operand.collect_fields(found),
            Self::IsNull(field)
            | Self::IsNotNull(field)
            | Self::Eq(field, _)
            | Self::Ne(field, _)
            | Self::Lt(field, _)
            | Self::Lte(field, _)
            | Self::Gt(field, _)
            | Self::Gte(field, _)
            | Self::In(field, _)
            | Self::NotIn(field, _)
            | Self::Between(field, _, _)
            | Self::Contains(field, _)
            | Self::StartsWith(field, _)
            | Self::Like(field, _) => single(field, found),
        }
    }
}

/// Compile a filter against an object type.
///
/// This is the fail-closed boundary the runtime needs: a filter naming a
/// property the object type does not declare is an error, not a predicate that
/// quietly matches every row.
pub fn compile_filter(
    object_type: &CompiledObjectType,
    filter: &Filter,
) -> Result<CompiledFilter, Error> {
    match filter.op {
        FilterOp::And | FilterOp::Or => {
            let logical = filter.op;
            if filter.args.is_empty() {
                // An empty `AND` is vacuously true and an empty `OR` is
                // vacuously false. Both are almost always an authoring bug, and
                // the first one fails open, so neither is accepted.
                return Err(Error::validation(format!(
                    "{} filter has no operands",
                    logical_name(logical)
                )));
            }
            let compiled: Result<Vec<CompiledFilter>, Error> = filter
                .args
                .iter()
                .map(|operand| compile_filter(object_type, operand))
                .collect();
            let compiled: Arc<[CompiledFilter]> = compiled?.into();
            Ok(match logical {
                FilterOp::Or => CompiledFilter::Or(compiled),
                _ => CompiledFilter::And(compiled),
            })
        }
        FilterOp::Not => {
            let [operand] = filter.args.as_slice() else {
                return Err(Error::validation(format!(
                    "not filter requires exactly one operand, got {}",
                    filter.args.len()
                )));
            };
            Ok(CompiledFilter::Not(Arc::new(compile_filter(
                object_type,
                operand,
            )?)))
        }
        scalar => {
            let name = filter.field.as_ref().ok_or_else(|| {
                Error::validation(format!("{} filter requires a field", logical_name(scalar)))
            })?;
            // Fail closed: a filter naming a property the object type does not
            // declare must not compile into one that matches every row.
            let property = object_type
                .property(name)
                .ok_or_else(|| Error::unknown_property(object_type, name))?;
            let field = FieldRef::new(property.id(), name.clone());
            let operand = Operation::of(filter, field)?;
            Ok(operand.compile(scalar))
        }
    }
}

/// The name a filter operator is shown under in diagnostics.
fn logical_name(op: FilterOp) -> &'static str {
    match op {
        FilterOp::And => "and",
        FilterOp::Or => "or",
        FilterOp::Not => "not",
        FilterOp::IsNull => "is_null",
        FilterOp::IsNotNull => "is_not_null",
        FilterOp::Eq => "eq",
        FilterOp::Ne => "ne",
        FilterOp::Lt => "lt",
        FilterOp::Lte => "lte",
        FilterOp::Gt => "gt",
        FilterOp::Gte => "gte",
        FilterOp::In => "in",
        FilterOp::NotIn => "not_in",
        FilterOp::Between => "between",
        FilterOp::Contains => "contains",
        FilterOp::StartsWith => "starts_with",
        FilterOp::Like => "like",
    }
}

/// The operand a scalar filter carries, validated for its operator.
enum Operation {
    /// No operand: `IS NULL`, `IS NOT NULL`.
    None(FieldRef),
    /// Exactly one scalar operand.
    One(FieldRef, Key),
    /// A non-empty set of scalar operands.
    Many(FieldRef, Vec<Key>),
    /// An inclusive pair of operands.
    Pair(FieldRef, Key, Key),
    /// A string operand.
    Text(FieldRef, Arc<str>),
    /// A compiled `LIKE` pattern.
    Pattern(FieldRef, Arc<LikePlan>),
}

impl Operation {
    fn of(filter: &Filter, field: FieldRef) -> Result<Self, Error> {
        match filter.op {
            FilterOp::IsNull | FilterOp::IsNotNull => Ok(Self::None(field)),
            FilterOp::Eq
            | FilterOp::Ne
            | FilterOp::Lt
            | FilterOp::Lte
            | FilterOp::Gt
            | FilterOp::Gte => {
                let value = filter.value.as_ref().ok_or_else(|| {
                    Error::validation(format!(
                        "{} filter requires a value",
                        logical_name(filter.op)
                    ))
                })?;
                Ok(Self::One(field, scalar_key(value, filter.op)?))
            }
            FilterOp::In | FilterOp::NotIn => {
                if filter.values.is_empty() {
                    return Err(Error::validation(format!(
                        "{} filter requires at least one value",
                        logical_name(filter.op)
                    )));
                }
                let keys = filter
                    .values
                    .iter()
                    .map(|value| scalar_key(value, filter.op))
                    .collect::<Result<Vec<Key>, Error>>()?;
                Ok(Self::Many(field, keys))
            }
            FilterOp::Between => {
                let [low, high] = filter.values.as_slice() else {
                    return Err(Error::validation(format!(
                        "between filter requires exactly two values, got {}",
                        filter.values.len()
                    )));
                };
                Ok(Self::Pair(
                    field,
                    scalar_key(low, filter.op)?,
                    scalar_key(high, filter.op)?,
                ))
            }
            FilterOp::Contains | FilterOp::StartsWith => {
                let text = text_operand(filter)?;
                Ok(Self::Text(field, text))
            }
            FilterOp::Like => {
                let pattern = text_operand(filter)?;
                let plan = LikePlan::new(&pattern).map_err(|error| {
                    Error::validation(format!("invalid LIKE pattern '{pattern}': {error}"))
                })?;
                Ok(Self::Pattern(field, Arc::new(plan)))
            }
            FilterOp::And | FilterOp::Or | FilterOp::Not => {
                unreachable!("logical operators are handled before operands are read")
            }
        }
    }

    fn compile(self, op: FilterOp) -> CompiledFilter {
        match self {
            Self::None(field) => match op {
                FilterOp::IsNull => CompiledFilter::IsNull(field),
                _ => CompiledFilter::IsNotNull(field),
            },
            Self::One(field, key) => match op {
                FilterOp::Eq => CompiledFilter::Eq(field, key),
                FilterOp::Ne => CompiledFilter::Ne(field, key),
                FilterOp::Lt => CompiledFilter::Lt(field, key),
                FilterOp::Lte => CompiledFilter::Lte(field, key),
                FilterOp::Gt => CompiledFilter::Gt(field, key),
                _ => CompiledFilter::Gte(field, key),
            },
            Self::Many(field, keys) => {
                let set: Arc<HashSet<Key>> = Arc::new(keys.into_iter().collect());
                match op {
                    FilterOp::NotIn => CompiledFilter::NotIn(field, set),
                    _ => CompiledFilter::In(field, set),
                }
            }
            Self::Pair(field, low, high) => CompiledFilter::Between(field, low, high),
            Self::Text(field, text) => match op {
                FilterOp::StartsWith => CompiledFilter::StartsWith(field, text),
                _ => CompiledFilter::Contains(field, text),
            },
            Self::Pattern(field, plan) => CompiledFilter::Like(field, plan),
        }
    }
}

/// Project a filter value onto a [`Key`], rejecting shapes that cannot match.
///
/// A [`Key`] is total over [`Value`], so this cannot fail on type alone — but an
/// array or object compared against a scalar property encodes to opaque bytes
/// that would silently never match, which is a modelling error worth reporting.
fn scalar_key(value: &Value, op: FilterOp) -> Result<Key, Error> {
    let key = Key::from(value);
    if matches!(key, Key::Bytes(_)) {
        return Err(Error::validation(format!(
            "{} filter compares a property against {} value",
            logical_name(op),
            key.type_name()
        )));
    }
    Ok(key)
}

/// Read the string operand of `LIKE`, `CONTAINS` or `STARTS WITH`.
fn text_operand(filter: &Filter) -> Result<Arc<str>, Error> {
    filter
        .value
        .as_ref()
        .and_then(Value::as_str)
        .map(Arc::from)
        .ok_or_else(|| {
            Error::validation(format!(
                "{} filter requires a string value",
                logical_name(filter.op)
            ))
        })
}

#[cfg(test)]
#[path = "filter_tests.rs"]
mod tests;
