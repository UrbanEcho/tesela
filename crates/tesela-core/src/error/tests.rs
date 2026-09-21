use super::*;
#[test]
fn retryability_requires_both_a_retryable_code_and_no_side_effect() {
    let transient = Error::transient("connection reset");
    assert!(transient.is_retryable());
    assert!(transient.side_effect().allows_retry());

    let after_write =
        Error::transient("connection reset").with_side_effect(SideEffectStatus::Unknown);
    assert!(
        !after_write.is_retryable(),
        "an unknown side effect must block an automatic retry"
    );

    let invalid = Error::validation("bad spec");
    assert!(!invalid.is_retryable());
}

#[test]
fn codes_round_trip_through_their_string_form() {
    for code in [
        ErrorCode::Validation,
        ErrorCode::StaleRevision,
        ErrorCode::BudgetExhausted,
        ErrorCode::PolicyDenied,
    ] {
        let rendered = code.as_str();
        let parsed: ErrorCode =
            serde_json::from_value(serde_json::Value::String(rendered.to_string()))
                .expect("code should deserialize from its string form");
        assert_eq!(parsed, code);
    }
}

#[test]
fn display_surfaces_an_unknown_side_effect() {
    let error = Error::adapter("write failed").with_side_effect(SideEffectStatus::Unknown);
    assert!(error.to_string().contains("side effects unknown"));
}
