use super::*;

#[test]
fn context_builder_requires_an_explicit_request_id() {
    assert!(ExecutionContext::builder("agent").build().is_err());
    assert!(
        ExecutionContext::builder("")
            .request_id("request")
            .build()
            .is_err()
    );
    let context = ExecutionContext::builder("agent")
        .request_id("request")
        .build()
        .expect("explicit request id");
    assert_eq!(context.request_id(), "request");
}

#[test]
fn budgets_fail_before_overcharging() {
    let context = ExecutionContext::new(
        Principal::new("agent"),
        Vec::new(),
        "request",
        None,
        CancellationToken::default(),
        ExecutionBudget {
            rows: Some(2),
            ..ExecutionBudget::default()
        },
    )
    .expect("context");
    assert!(context.charge_rows(2).is_ok());
    assert!(context.charge_rows(1).is_err());
}

#[test]
fn delegation_must_attenuate_and_end_at_the_principal() {
    let resource = ApiName::new_unchecked("customer");
    let root = AuthorityGrant {
        issuer: Arc::from("host"),
        delegate: Arc::from("broker"),
        scopes: Arc::from([AuthorityScope {
            operation: Operation::Read,
            resource: None,
        }]),
        expires_at_unix_ms: None,
        max_calls: Some(4),
        delegation_depth: 2,
    };
    let child = AuthorityGrant {
        issuer: Arc::from("broker"),
        delegate: Arc::from("agent"),
        scopes: Arc::from([AuthorityScope {
            operation: Operation::Read,
            resource: Some(resource.clone()),
        }]),
        expires_at_unix_ms: None,
        max_calls: Some(2),
        delegation_depth: 1,
    };
    let context = ExecutionContext::new(
        Principal::new("agent"),
        vec![root.clone(), child],
        "request",
        None,
        CancellationToken::default(),
        ExecutionBudget::default(),
    )
    .expect("valid chain");
    assert!(context.check(Operation::Read, &resource).is_ok());
    let expanded = AuthorityGrant {
        max_calls: Some(5),
        ..root.clone()
    };
    assert!(root.validate_child(&expanded).is_err());
    assert!(
        ExecutionContext::new(
            Principal::new("other"),
            vec![root],
            "request",
            None,
            CancellationToken::default(),
            ExecutionBudget::default()
        )
        .is_err()
    );
}

#[test]
fn cancellation_and_deadlines_fail_before_authorization() {
    let token = CancellationToken::default();
    token.cancel();
    let cancelled = ExecutionContext::new(
        Principal::new("agent"),
        Vec::new(),
        "cancelled",
        None,
        token,
        ExecutionBudget::default(),
    )
    .expect("context");
    assert!(
        cancelled
            .check(Operation::Read, &ApiName::from("customer"))
            .is_err_and(|error| error.is(crate::ErrorCode::Cancelled))
    );
    let expired = ExecutionContext::new(
        Principal::new("agent"),
        Vec::new(),
        "expired",
        Some(Instant::now() - Duration::from_millis(1)),
        CancellationToken::default(),
        ExecutionBudget::default(),
    )
    .expect("context");
    assert!(
        expired
            .check(Operation::Read, &ApiName::from("customer"))
            .is_err_and(|error| error.is(crate::ErrorCode::Timeout))
    );
}

#[test]
fn byte_and_external_cost_budgets_are_bounded() {
    let context = ExecutionContext::new(
        Principal::new("agent"),
        Vec::new(),
        "cost",
        None,
        CancellationToken::default(),
        ExecutionBudget {
            bytes: Some(8),
            external_cost: Some(2),
            ..ExecutionBudget::default()
        },
    )
    .expect("context");
    assert!(context.charge_bytes(8).is_ok());
    assert!(context.charge_bytes(1).is_err());
    assert!(context.charge_external_cost(2).is_ok());
    assert!(context.charge_external_cost(1).is_err());
}
