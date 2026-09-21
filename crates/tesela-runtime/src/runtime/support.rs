use super::*;

pub(super) fn object_type<'a>(
    snapshot: &'a ExecutionSnapshot,
    name: &ApiName,
) -> Result<&'a CompiledObjectType, Error> {
    snapshot
        .ontology
        .object_type(name)
        .ok_or_else(|| Error::unknown_object_type(name))
}
pub(super) fn object_backend<'a>(
    snapshot: &'a ExecutionSnapshot,
    object: &CompiledObjectType,
) -> Result<&'a Arc<dyn Backend>, Error> {
    snapshot
        .objects
        .get(object.id().index())
        .and_then(Option::as_ref)
        .ok_or_else(|| Error::unsupported(format!("object '{}' is not bound", object.api_name())))
}
pub(super) fn effective_limit(requested: Option<i32>, maximum: usize) -> usize {
    requested.map_or(maximum, |limit| {
        usize::try_from(limit.max(0)).unwrap_or(0).min(maximum)
    })
}

pub(super) fn ensure_approval(
    context: &ExecutionContext,
    resource: &ApiName,
    decision: &PolicyDecision,
) -> Result<(), Error> {
    if decision.require_approval && !context.is_approved(resource) {
        Err(Error::new(
            ErrorCode::ApprovalRequired,
            format!("operation on '{resource}' requires approval"),
        ))
    } else {
        Ok(())
    }
}

pub(super) fn check_query(object: &CompiledObjectType, query: &Query) -> Result<(), Error> {
    if let Some(filter) = &query.filter {
        compile_filter(object, filter)?;
    }
    for sort in &query.sort {
        if object.property(&sort.property).is_none() {
            return Err(Error::unknown_property(object, &sort.property));
        }
    }
    Ok(())
}

pub(super) fn apply_mutation(
    context: &ExecutionContext,
    ontology: &CompiledOntology,
    backend: &dyn Backend,
    object: &CompiledObjectType,
    mutation: Mutation,
) -> Result<MutationResult, Error> {
    match mutation {
        Mutation::Create { mut values } => {
            object.apply_defaults(&mut values);
            ontology.validate_create(object, &values)?;
            let key = primary_key(object, &values)?;
            backend.create(context, object.api_name(), key, values)
        }
        Mutation::Update {
            primary_key,
            values,
            expected_revision,
            ..
        } => {
            ontology.validate_patch(object, &values)?;
            backend.update(
                context,
                object.api_name(),
                &primary_key,
                values,
                expected_revision.as_ref(),
            )
        }
        Mutation::Delete {
            primary_key,
            expected_revision,
            ..
        } => backend.delete(
            context,
            object.api_name(),
            &primary_key,
            expected_revision.as_ref(),
        ),
        Mutation::Upsert {
            mut values,
            expected_revision,
            ..
        } => {
            object.apply_defaults(&mut values);
            ontology.validate_create(object, &values)?;
            let key = primary_key(object, &values)?;
            backend.upsert(
                context,
                object.api_name(),
                key,
                values,
                expected_revision.as_ref(),
            )
        }
        Mutation::Batch { items } => backend.batch(
            context,
            object.api_name(),
            object.primary_key().api_name(),
            &items,
        ),
    }
}

pub(super) fn validate_mutation(
    ontology: &CompiledOntology,
    object: &CompiledObjectType,
    mutation: &Mutation,
) -> Result<(), Error> {
    match mutation {
        Mutation::Create { values } | Mutation::Upsert { values, .. } => {
            let mut values = values.clone();
            object.apply_defaults(&mut values);
            ontology.validate_create(object, &values)?;
            primary_key(object, &values)?;
        }
        Mutation::Update { values, .. } => ontology.validate_patch(object, values)?,
        Mutation::Delete { .. } => {}
        Mutation::Batch { items } => {
            if items.is_empty() {
                return Err(Error::bad_request("batch requires at least one mutation"));
            }
            for item in items {
                if matches!(item, Mutation::Batch { .. }) {
                    return Err(Error::bad_request("nested batches are not supported"));
                }
                validate_mutation(ontology, object, item)?;
            }
        }
    }
    Ok(())
}

pub(super) fn primary_key(
    object: &CompiledObjectType,
    values: &BTreeMap<ApiName, Value>,
) -> Result<Key, Error> {
    let field = object.primary_key().api_name();
    Key::identity(values.get(field).ok_or_else(|| {
        Error::bad_request(format!("object '{object}' requires primary key '{field}'"))
    })?)
    .map_err(|error| Error::bad_request(format!("{object}.{field}: {error}")))
}

pub(super) fn value_fields(input: &Value) -> Result<BTreeMap<ApiName, Value>, Error> {
    input
        .as_object()
        .ok_or_else(|| Error::bad_request("action input must be an object"))?
        .iter()
        .map(|(name, value)| {
            Ok((
                ApiName::new(name).map_err(|error| Error::bad_request(error.to_string()))?,
                Value::new(value.clone()),
            ))
        })
        .collect()
}

pub(super) fn redact(records: &mut [Record], fields: &[ApiName]) {
    for record in records {
        for field in fields {
            record.values.remove(field);
        }
    }
}

pub(super) fn value_bytes(value: &Value) -> u64 {
    json_bytes(&value.0)
}

fn json_bytes(value: &serde_json::Value) -> u64 {
    match value {
        serde_json::Value::Null => 4,
        serde_json::Value::Bool(_) => 5,
        serde_json::Value::Number(_) => 24,
        serde_json::Value::String(value) => value.len() as u64,
        serde_json::Value::Array(values) => values.iter().map(json_bytes).sum(),
        serde_json::Value::Object(values) => values
            .iter()
            .map(|(name, value)| name.len() as u64 + json_bytes(value))
            .sum(),
    }
}

pub(super) fn fields_bytes(values: &BTreeMap<ApiName, Value>) -> u64 {
    values
        .iter()
        .map(|(name, value)| name.as_str().len() as u64 + value_bytes(value))
        .sum()
}

pub(super) fn records_bytes(records: &[Record]) -> u64 {
    records
        .iter()
        .map(|record| value_bytes(&record.primary_key.to_value()) + fields_bytes(&record.values))
        .sum()
}

pub(super) fn validate_schema(schema: &Value, input: &Value) -> Result<(), Error> {
    validate_schema_at(&schema.0, &input.0, "$")
}

fn validate_schema_at(
    schema: &serde_json::Value,
    input: &serde_json::Value,
    path: &str,
) -> Result<(), Error> {
    let Some(schema) = schema.as_object() else {
        return Err(Error::validation("action schema must be an object"));
    };
    if let Some(expected) = schema.get("type").and_then(serde_json::Value::as_str) {
        let valid = match expected {
            "object" => input.is_object(),
            "array" => input.is_array(),
            "string" | "uuid" | "date" | "timestamp" | "timestamptz" => input.is_string(),
            "integer" | "bigint" => input.as_i64().is_some() || input.as_u64().is_some(),
            "number" | "float" => input.is_number(),
            "decimal" => input.is_number() || input.is_string(),
            "boolean" => input.is_boolean(),
            "json" => true,
            "enum" => !input.is_array() && !input.is_object(),
            "null" => input.is_null(),
            other => {
                return Err(Error::validation(format!(
                    "unsupported schema type '{other}'"
                )));
            }
        };
        if !valid {
            return Err(Error::bad_request(format!(
                "action input at {path} must be {expected}"
            )));
        }
    }
    if let Some(allowed) = schema.get("enum").and_then(serde_json::Value::as_array)
        && !allowed.contains(input)
    {
        return Err(Error::bad_request(format!(
            "action input at {path} is outside the allowed values"
        )));
    }
    if let Some(object) = input.as_object() {
        if let Some(required) = schema.get("required").and_then(serde_json::Value::as_array) {
            for field in required.iter().filter_map(serde_json::Value::as_str) {
                if !object.contains_key(field) {
                    return Err(Error::bad_request(format!(
                        "action input requires {path}.{field}"
                    )));
                }
            }
        }
        if let Some(properties) = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
        {
            for (field, field_schema) in properties {
                if let Some(value) = object.get(field) {
                    validate_schema_at(field_schema, value, &format!("{path}.{field}"))?;
                }
            }
        }
    }
    if let (Some(items), Some(values)) = (schema.get("items"), input.as_array()) {
        for (index, value) in values.iter().enumerate() {
            validate_schema_at(items, value, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

#[derive(Clone)]
pub(super) struct AggregateState {
    count: u64,
    sum: f64,
    numeric_count: u64,
    min: Option<Value>,
    max: Option<Value>,
}
impl Default for AggregateState {
    fn default() -> Self {
        Self {
            count: 0,
            sum: 0.0,
            numeric_count: 0,
            min: None,
            max: None,
        }
    }
}

pub(super) struct AggregateAccumulator<'a> {
    query: &'a AggregateQuery,
    visible_groups: Vec<ApiName>,
    groups: HashMap<Vec<Key>, Vec<AggregateState>>,
}

impl<'a> AggregateAccumulator<'a> {
    pub(super) fn new(query: &'a AggregateQuery, redactions: &[ApiName]) -> Self {
        Self {
            query,
            visible_groups: query
                .group_by
                .iter()
                .filter(|field| !redactions.contains(field))
                .cloned()
                .collect(),
            groups: HashMap::new(),
        }
    }

    pub(super) fn extend(&mut self, records: &[Record]) {
        for record in records {
            let key: Vec<Key> = self
                .visible_groups
                .iter()
                .map(|field| record.values.get(field).map_or(Key::Null, Key::from))
                .collect();
            let states = self
                .groups
                .entry(key)
                .or_insert_with(|| vec![AggregateState::default(); self.query.aggregations.len()]);
            for (aggregation, state) in self.query.aggregations.iter().zip(states) {
                state.count += 1;
                if let Some(value) = aggregation
                    .property
                    .as_ref()
                    .and_then(|field| record.values.get(field))
                {
                    if let Some(number) = value.as_f64() {
                        state.sum += number;
                        state.numeric_count += 1;
                    }
                    if state.min.as_ref().is_none_or(|current| value < current) {
                        state.min = Some(value.clone());
                    }
                    if state.max.as_ref().is_none_or(|current| value > current) {
                        state.max = Some(value.clone());
                    }
                }
            }
        }
    }

    pub(super) fn finish(mut self) -> AggregateResult {
        if self.groups.is_empty() && self.visible_groups.is_empty() {
            self.groups.insert(
                Vec::new(),
                vec![AggregateState::default(); self.query.aggregations.len()],
            );
        }
        let mut groups: Vec<_> = self.groups.into_iter().collect();
        groups.sort_by(|left, right| left.0.cmp(&right.0));
        AggregateResult {
            groups: groups
                .into_iter()
                .map(|(key, states)| {
                    let mut row = BTreeMap::new();
                    for (field, value) in self.visible_groups.iter().zip(key) {
                        row.insert(field.to_string(), value.to_value());
                    }
                    for (aggregation, state) in self.query.aggregations.iter().zip(states) {
                        let value = match aggregation.function {
                            AggregationFunction::Count => Value::integer(state.count as i64),
                            AggregationFunction::Sum => Value::float(state.sum),
                            AggregationFunction::Avg if state.numeric_count > 0 => {
                                Value::float(state.sum / state.numeric_count as f64)
                            }
                            AggregationFunction::Avg => Value::null(),
                            AggregationFunction::Min => state.min.unwrap_or_default(),
                            AggregationFunction::Max => state.max.unwrap_or_default(),
                        };
                        row.insert(aggregation.alias.clone(), value);
                    }
                    row
                })
                .collect(),
        }
    }
}
