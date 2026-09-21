use super::lower::*;
use super::*;

/// The spec compiler: one pass, collecting every problem it finds.
pub(super) struct Compiler<'a> {
    spec: &'a Spec,
    diagnostics: Diagnostics,
}

impl<'a> Compiler<'a> {
    pub(super) fn new(spec: &'a Spec) -> Self {
        Self {
            spec,
            diagnostics: Diagnostics::new(),
        }
    }

    /// Compile, or report everything wrong.
    pub(super) fn run(mut self) -> Result<CompiledOntology, SpecError> {
        let fingerprint = match self.spec.fingerprint() {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                return Err(SpecError::single(Diagnostic::error(
                    DiagnosticCode::ValidationFailed,
                    error.to_string(),
                )));
            }
        };

        self.check_version();
        self.check_duplicate_names();

        let traits = self.compile_traits();
        let (object_types, object_type_index) = self.compile_object_types(&traits);
        let (link_types, link_type_index) = self.compile_link_types(&object_type_index);
        let (actions, action_index) = self.compile_actions(&object_type_index);

        if self.diagnostics.has_errors() {
            return Err(SpecError::new(self.diagnostics.into_vec()));
        }

        let trait_index = self
            .spec
            .traits
            .iter()
            .enumerate()
            .map(|(index, item)| (item.api_name.clone(), index))
            .collect();

        Ok(CompiledOntology {
            spec: Arc::new(self.spec.clone()),
            fingerprint,
            generation: 0,
            object_types,
            object_type_index,
            link_types,
            link_type_index,
            actions,
            action_index,
            traits,
            trait_index,
        })
    }

    /// Reject a spec whose declared version this build cannot compile.
    fn check_version(&mut self) {
        let version = self.spec.version.as_str();
        if version != SPEC_VERSION {
            self.diagnostics.push(Diagnostic::error(
                DiagnosticCode::ValidationFailed,
                format!(
                    "unsupported spec version '{version}'; this build compiles '{SPEC_VERSION}'"
                ),
            ));
        }
    }

    /// Reject a spec that declares the same name twice.
    ///
    /// A duplicate would make the name index arbitrary, so it is an error rather
    /// than a last-one-wins: which one won would depend on declaration order,
    /// and nothing in the spec says which one the author meant.
    fn check_duplicate_names(&mut self) {
        for (kind, names) in duplicate_names(self.spec) {
            for name in names {
                self.diagnostics.push(
                    Diagnostic::error(
                        DiagnosticCode::DuplicateName,
                        format!("{} '{name}' is declared more than once", kind.as_str()),
                    )
                    .with_api_name(name),
                );
            }
        }
    }

    /// Compile traits, validating the properties they contribute.
    fn compile_traits(&mut self) -> Vec<CompiledTrait> {
        let spec = self.spec;
        let mut compiled = Vec::with_capacity(spec.traits.len());
        for item in spec.traits.iter() {
            self.check_properties(&item.api_name, "trait", &item.properties);
            compiled.push(CompiledTrait {
                api_name: item.api_name.clone(),
                display: item.display.clone(),
                description: item.description.clone(),
                properties: compile_properties(&item.properties, &PropertyOrigin::Declared),
            });
        }
        compiled
    }

    /// Compile object types, flattening traits into them.
    fn compile_object_types(
        &mut self,
        traits: &[CompiledTrait],
    ) -> (Vec<CompiledObjectType>, HashMap<ApiName, ObjectTypeId>) {
        let spec = self.spec;
        let mut compiled = Vec::with_capacity(spec.object_types.len());
        let mut index = HashMap::with_capacity(spec.object_types.len());

        for (position, item) in spec.object_types.iter().enumerate() {
            let id = ObjectTypeId::new(position as u32);
            index.insert(item.api_name.clone(), id);

            self.check_properties(&item.api_name, "object type", &item.properties);

            let mut properties: Vec<CompiledProperty> = Vec::new();
            let mut names: HashMap<ApiName, ()> = HashMap::new();
            for property in &item.properties {
                if names.insert(property.api_name.clone(), ()).is_none() {
                    properties.push(compile_property(
                        PropertyId::new(properties.len() as u32),
                        property,
                        PropertyOrigin::Declared,
                    ));
                }
            }

            // Flattening is where inheritance stops being a runtime concern.
            for trait_name in &item.traits {
                let Some(definition) = traits.iter().find(|item| item.api_name == *trait_name)
                else {
                    self.diagnostics.push(
                        Diagnostic::error(
                            DiagnosticCode::BrokenReference,
                            format!(
                                "object type '{}' declares trait '{trait_name}', which the spec does not define",
                                item.api_name
                            ),
                        )
                        .with_api_name(item.api_name.clone()),
                    );
                    continue;
                };
                for property in &definition.properties {
                    if names.insert(property.api_name.clone(), ()).is_some() {
                        self.diagnostics.push(
                            Diagnostic::error(
                                DiagnosticCode::InvalidObjectType,
                                format!(
                                    "property '{}' on object type '{}' is declared more than once: it comes from both {}",
                                    property.api_name, item.api_name, PropertyOrigin::Trait(trait_name.clone())
                                ),
                            )
                            .with_api_name(item.api_name.clone()),
                        );
                        continue;
                    }
                    properties.push(adopt_property(
                        property,
                        PropertyId::new(properties.len() as u32),
                        PropertyOrigin::Trait(trait_name.clone()),
                    ));
                }
            }

            let primary_key =
                primary_key_id(&item.primary_key, &properties, item, &mut self.diagnostics);

            compiled.push(CompiledObjectType {
                id,
                api_name: item.api_name.clone(),
                display: item.display.clone(),
                description: item.description.clone(),
                primary_key,
                properties,
                property_index: HashMap::new(),
                traits: item.traits.clone(),
                tags: item.tags.clone(),
                deprecated_at: item.deprecated_at.clone(),
            });
        }

        for object_type in &mut compiled {
            object_type.property_index = object_type
                .properties
                .iter()
                .enumerate()
                .map(|(index, property)| (property.api_name.clone(), PropertyId::new(index as u32)))
                .collect();
        }

        (compiled, index)
    }

    /// Compile link types, resolving endpoints and checking inverse symmetry.
    fn compile_link_types(
        &mut self,
        object_types: &HashMap<ApiName, ObjectTypeId>,
    ) -> (Vec<CompiledLinkType>, HashMap<ApiName, LinkId>) {
        let spec = self.spec;
        let mut index: HashMap<ApiName, LinkId> = HashMap::with_capacity(spec.link_types.len());
        for (position, item) in spec.link_types.iter().enumerate() {
            index.insert(item.api_name.clone(), LinkId::new(position as u32));
        }

        let mut compiled = Vec::with_capacity(spec.link_types.len());
        for (position, item) in spec.link_types.iter().enumerate() {
            let from = self.resolve_endpoint(item, &item.from, "from", object_types);
            let to = self.resolve_endpoint(item, &item.to, "to", object_types);

            let inverse = match &item.inverse {
                None => None,
                Some(name) => match index.get(name) {
                    None => {
                        self.diagnostics.push(
                            Diagnostic::error(
                                DiagnosticCode::BrokenReference,
                                format!(
                                    "link type '{}' declares inverse '{name}', which the spec does not define",
                                    item.api_name
                                ),
                            )
                            .with_api_name(item.api_name.clone()),
                        );
                        None
                    }
                    Some(id) => {
                        let other = &spec.link_types[id.index()];
                        if other.from != item.to || other.to != item.from {
                            self.diagnostics.push(
                                Diagnostic::error(
                                    DiagnosticCode::InvalidLink,
                                    format!(
                                        "link type '{}' ({:?} -> {:?}) is declared the inverse of '{name}' ({:?} -> {:?}), but an inverse must run the other way between the same two object types",
                                        item.api_name, item.from, item.to, other.from, other.to
                                    ),
                                )
                                .with_api_name(item.api_name.clone()),
                            );
                        }
                        // Reported per link, not per pair: the fault is in this
                        // link's own declaration, so two links that each name a
                        // different non-reciprocating inverse are two faults and
                        // get two messages.
                        let mutual = other.inverse.as_ref() == Some(&item.api_name);
                        if !mutual {
                            self.diagnostics.push(
                                Diagnostic::error(
                                    DiagnosticCode::InvalidLink,
                                    format!(
                                        "link type '{}' declares '{name}' as its inverse, but '{name}' does not declare '{}' as its own",
                                        item.api_name, item.api_name
                                    ),
                                )
                                .with_api_name(item.api_name.clone()),
                            );
                        }
                        Some(*id)
                    }
                },
            };

            if from.is_none() || to.is_none() {
                // An unresolved endpoint means no id to compile against;
                // `from`/`to` default to the first object type only so the
                // record below can be built. The spec is rejected anyway.
                let fallback = ObjectTypeId::new(0);
                compiled.push(CompiledLinkType {
                    id: LinkId::new(position as u32),
                    api_name: item.api_name.clone(),
                    display: item.display.clone(),
                    from: from.unwrap_or(fallback),
                    to: to.unwrap_or(fallback),
                    cardinality: item.cardinality,
                    inverse,
                    deprecated_at: item.deprecated_at.clone(),
                });
                continue;
            }

            compiled.push(CompiledLinkType {
                id: LinkId::new(position as u32),
                api_name: item.api_name.clone(),
                display: item.display.clone(),
                from: from.expect("checked above"),
                to: to.expect("checked above"),
                cardinality: item.cardinality,
                inverse,
                deprecated_at: item.deprecated_at.clone(),
            });
        }

        (compiled, index)
    }

    /// Compile actions, resolving their subject.
    fn compile_actions(
        &mut self,
        object_types: &HashMap<ApiName, ObjectTypeId>,
    ) -> (Vec<CompiledAction>, HashMap<ApiName, ActionId>) {
        let spec = self.spec;
        let mut compiled = Vec::with_capacity(spec.actions.len());
        let mut index = HashMap::with_capacity(spec.actions.len());

        for (position, item) in spec.actions.iter().enumerate() {
            let id = ActionId::new(position as u32);
            index.insert(item.api_name.clone(), id);

            let subject = item.subject.as_ref().and_then(|name| {
                let resolved = object_types.get(name).copied();
                if resolved.is_none() {
                    self.diagnostics.push(
                        Diagnostic::error(
                            DiagnosticCode::BrokenReference,
                            format!(
                                "action '{}' has subject '{name}', which the spec does not define",
                                item.api_name
                            ),
                        )
                        .with_api_name(item.api_name.clone()),
                    );
                }
                resolved
            });

            if item.risk_level == RiskLevel::High && item.effects.is_read_only() {
                self.diagnostics.push(
                    Diagnostic::warning(
                        DiagnosticCode::InvalidAction,
                        format!(
                            "action '{}' is declared high risk but has no write or external effect",
                            item.api_name
                        ),
                    )
                    .with_api_name(item.api_name.clone()),
                );
            }

            compiled.push(CompiledAction {
                id,
                api_name: item.api_name.clone(),
                display: item.display.clone(),
                description: item.description.clone(),
                subject,
                input_schema: item.input_schema.clone(),
                output_schema: item.output_schema.clone(),
                risk_level: item.risk_level,
                effects: item.effects,
                reads: item.reads.clone(),
                writes: item.writes.clone(),
                creates: item.creates.clone(),
                external_effects: item.external_effects.clone(),
                approval: item.approval,
                requires_revision: item.requires_revision,
                preconditions: item.preconditions.clone(),
                concurrency: item.concurrency,
                idempotency_key: item.idempotency_key.clone(),
                deprecated_at: item.deprecated_at.clone(),
            });
        }

        (compiled, index)
    }

    /// Resolve one end of a link type.
    fn resolve_endpoint(
        &mut self,
        link: &LinkType,
        name: &ApiName,
        side: &str,
        object_types: &HashMap<ApiName, ObjectTypeId>,
    ) -> Option<ObjectTypeId> {
        let resolved = object_types.get(name).copied();
        if resolved.is_none() {
            self.diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::BrokenReference,
                    format!(
                        "link type '{}' has {side} object type '{name}', which the spec does not define",
                        link.api_name
                    ),
                )
                .with_api_name(link.api_name.clone()),
            );
        }
        resolved
    }

    /// Validate a list of properties, reporting duplicates and bad declarations.
    fn check_properties(&mut self, owner: &ApiName, owner_kind: &str, properties: &[Property]) {
        let mut seen: HashMap<&ApiName, ()> = HashMap::with_capacity(properties.len());
        for property in properties {
            if seen.insert(&property.api_name, ()).is_some() {
                self.diagnostics.push(
                    Diagnostic::error(
                        DiagnosticCode::DuplicateName,
                        format!(
                            "{owner_kind} '{owner}' declares property '{}' more than once",
                            property.api_name
                        ),
                    )
                    .with_api_name(owner.clone()),
                );
            }
            for diagnostic in check_property(owner, owner_kind, property) {
                self.diagnostics.push(diagnostic);
            }
        }
    }
}
