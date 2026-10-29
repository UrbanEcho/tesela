//! Capability projection: which operations a principal may call, how each is exposed, and its hints.

use tesela_core::RiskLevel;
use tesela_ir::ActionEffects;

use crate::capability::{CapabilityAnnotations, contains_phrase, tokenize};

use super::schema::{
    action_schema, aggregate_schema, get_schema, mutation_schema, search_schema, traverse_schema,
};
use super::*;

/// Per-suffix mutation metadata. Risk, idempotency and revision needs are
/// fixed by the operation's semantics, not by the host.
struct MutationProfile {
    suffix: &'static str,
    risk: RiskLevel,
    idempotent: bool,
    requires_revision: bool,
    verbs: &'static [&'static str],
}

const MUTATIONS: [MutationProfile; 4] = [
    MutationProfile {
        suffix: "create",
        risk: RiskLevel::Medium,
        idempotent: false,
        requires_revision: false,
        verbs: &["create", "add", "insert"],
    },
    MutationProfile {
        suffix: "update",
        risk: RiskLevel::Medium,
        idempotent: true,
        requires_revision: true,
        verbs: &["update", "edit", "change", "modify"],
    },
    MutationProfile {
        suffix: "delete",
        risk: RiskLevel::High,
        idempotent: true,
        requires_revision: true,
        verbs: &["delete", "remove"],
    },
    MutationProfile {
        suffix: "upsert",
        risk: RiskLevel::Medium,
        idempotent: true,
        requires_revision: false,
        verbs: &["upsert", "save", "put", "replace"],
    },
];

const SEARCH_VERBS: &[&str] = &["search", "list", "find", "query"];
const AGGREGATE_VERBS: &[&str] = &[
    "aggregate",
    "count",
    "sum",
    "average",
    "total",
    "group",
    "stats",
];
const GET_VERBS: &[&str] = &["get", "read", "fetch", "show", "lookup"];
const TRAVERSE_VERBS: &[&str] = &["traverse", "follow", "related", "linked"];

/// Behavioral hints derived from declared effects and risk.
fn annotations(effects: ActionEffects, risk: RiskLevel) -> CapabilityAnnotations {
    let read_only = effects.is_read_only();
    CapabilityAnnotations {
        read_only,
        destructive: !read_only && risk == RiskLevel::High,
        idempotent: effects.idempotent,
        open_world: effects.external,
    }
}

/// Direct when the task names the resource and one of the operation's verbs;
/// otherwise the fallback exposure. Whole-token matching keeps `cust` from
/// matching `customer`.
fn exposure(
    allowed: bool,
    task: &[String],
    resource: &str,
    verbs: &[&str],
    fallback: ToolExposure,
) -> ToolExposure {
    if !allowed {
        ToolExposure::Hidden
    } else if contains_phrase(task, &tokenize(resource))
        && verbs
            .iter()
            .any(|verb| task.iter().any(|token| token == verb))
    {
        ToolExposure::Direct
    } else {
        fallback
    }
}

impl Runtime {
    /// Compile the least capability surface available to this context.
    ///
    /// `task` is free text; its whole tokens decide which capabilities are
    /// `Direct`. An empty task yields no `Direct` capabilities.
    pub fn capabilities(
        &self,
        context: &ExecutionContext,
        task: &str,
    ) -> Result<CapabilityCatalog, Error> {
        let snapshot = self.snapshot.load_full();
        let task = tokenize(task);
        let mut capabilities = Vec::new();
        for object in snapshot.ontology.object_types() {
            let backend = snapshot
                .objects
                .get(object.id().index())
                .and_then(Option::as_ref);
            let resource = object.api_name();
            let description_of = |suffix: &str| format!("{suffix} {resource} objects");
            for (suffix, operation, supported, verbs) in [
                (
                    "search",
                    Operation::Search,
                    backend.is_some_and(|item| item.capabilities().search),
                    SEARCH_VERBS,
                ),
                (
                    "get",
                    Operation::Read,
                    backend.is_some_and(|item| item.capabilities().get),
                    GET_VERBS,
                ),
                (
                    "aggregate",
                    Operation::Aggregate,
                    backend.is_some_and(|item| {
                        item.capabilities().aggregate || item.capabilities().search
                    }),
                    AGGREGATE_VERBS,
                ),
            ] {
                let name = format!("{resource}.{suffix}");
                let decision = supported
                    .then(|| {
                        self.authorize(context, operation, ResourceKind::ObjectType, resource, None)
                    })
                    .and_then(Result::ok);
                let effects = ActionEffects {
                    reads: true,
                    writes: false,
                    external: false,
                    idempotent: true,
                };
                let input_schema = match suffix {
                    "search" => search_schema(object),
                    "aggregate" => aggregate_schema(object),
                    _ => get_schema(object),
                };
                let exposure = exposure(
                    decision.is_some(),
                    &task,
                    resource.as_str(),
                    verbs,
                    ToolExposure::Discoverable,
                );
                capabilities.push(Capability {
                    description: description_of(suffix),
                    name,
                    operation,
                    exposure,
                    input_schema: Value::new(input_schema),
                    effects,
                    annotations: annotations(effects, RiskLevel::Low),
                    approval_required: decision
                        .as_ref()
                        .is_some_and(|decision| decision.require_approval),
                    risk: RiskLevel::Low,
                    concurrency: None,
                    requires_revision: false,
                    preconditions: Vec::new(),
                });
            }
            if let Some(backend) = backend {
                let supports = backend.capabilities();
                for profile in &MUTATIONS {
                    let supported = match profile.suffix {
                        "create" => supports.create,
                        "update" => supports.update,
                        "delete" => supports.delete,
                        _ => supports.upsert,
                    };
                    let name = format!("{resource}.{}", profile.suffix);
                    let decision = supported
                        .then(|| {
                            self.authorize(
                                context,
                                Operation::Mutate,
                                ResourceKind::ObjectType,
                                resource,
                                None,
                            )
                        })
                        .and_then(Result::ok);
                    let effects = ActionEffects {
                        reads: false,
                        writes: true,
                        external: false,
                        idempotent: profile.idempotent,
                    };
                    let exposure = exposure(
                        decision.is_some(),
                        &task,
                        resource.as_str(),
                        profile.verbs,
                        ToolExposure::ProgramOnly,
                    );
                    capabilities.push(Capability {
                        description: description_of(profile.suffix),
                        name,
                        operation: Operation::Mutate,
                        exposure,
                        input_schema: Value::new(mutation_schema(object, profile.suffix)),
                        effects,
                        annotations: annotations(effects, profile.risk),
                        approval_required: decision
                            .as_ref()
                            .is_some_and(|decision| decision.require_approval),
                        risk: profile.risk,
                        concurrency: None,
                        requires_revision: profile.requires_revision,
                        preconditions: Vec::new(),
                    });
                }
            }
        }
        for link in snapshot.ontology.link_types() {
            let bound = snapshot
                .links
                .get(link.id().index())
                .is_some_and(Option::is_some);
            let decision = bound
                .then(|| {
                    self.authorize(
                        context,
                        Operation::Traverse,
                        ResourceKind::LinkType,
                        link.api_name(),
                        None,
                    )
                })
                .and_then(Result::ok);
            let effects = ActionEffects {
                reads: true,
                writes: false,
                external: false,
                idempotent: true,
            };
            let source = snapshot.ontology.object_type_by_id(link.from());
            let exposure = exposure(
                decision.is_some(),
                &task,
                link.api_name().as_str(),
                TRAVERSE_VERBS,
                ToolExposure::Discoverable,
            );
            capabilities.push(Capability {
                name: format!("{}.traverse", link.api_name()),
                description: format!("traverse the {} relationship", link.api_name()),
                operation: Operation::Traverse,
                exposure,
                input_schema: Value::new(traverse_schema(source)),
                effects,
                annotations: annotations(effects, RiskLevel::Low),
                approval_required: decision
                    .as_ref()
                    .is_some_and(|decision| decision.require_approval),
                risk: RiskLevel::Low,
                concurrency: None,
                requires_revision: false,
                preconditions: Vec::new(),
            });
        }
        for action in snapshot.ontology.actions() {
            let bound = snapshot
                .actions
                .get(action.id().index())
                .is_some_and(Option::is_some);
            let decision = bound
                .then(|| {
                    self.authorize(
                        context,
                        Operation::Execute,
                        ResourceKind::Action,
                        action.api_name(),
                        None,
                    )
                })
                .and_then(Result::ok);
            let name = action.api_name().to_string();
            let exposure = match decision {
                None => ToolExposure::Hidden,
                Some(_) if contains_phrase(&task, &tokenize(&name)) => ToolExposure::Direct,
                Some(_) => ToolExposure::Discoverable,
            };
            let approval_required = decision
                .as_ref()
                .is_some_and(|decision| decision.require_approval)
                || !action.is_read_only()
                || !matches!(action.approval(), tesela_ir::ApprovalPolicy::Never);
            capabilities.push(Capability {
                description: action
                    .description()
                    .unwrap_or("semantic action")
                    .to_string(),
                annotations: annotations(action.effects(), action.risk_level()),
                effects: action.effects(),
                name,
                operation: Operation::Execute,
                exposure,
                input_schema: action_schema(action.input_schema()),
                approval_required,
                risk: action.risk_level(),
                concurrency: Some(action.concurrency()),
                requires_revision: action.requires_revision(),
                preconditions: action.preconditions().to_vec(),
            });
        }
        // Handles only exist for a principal that can retain results, so a
        // principal with nothing visible sees no handle capabilities either.
        if capabilities
            .iter()
            .any(|capability| capability.exposure != ToolExposure::Hidden)
        {
            capabilities.extend(super::results::handle_capabilities());
        }
        Ok(CapabilityCatalog {
            generation: snapshot.ontology.generation(),
            fingerprint: snapshot.ontology.fingerprint().clone(),
            capabilities,
        })
    }
}
