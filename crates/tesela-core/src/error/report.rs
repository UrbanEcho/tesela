use serde::Serialize;

use super::{Error, ErrorCode, Recovery, SideEffectStatus};

/// An error reshaped for a model to read and act on.
///
/// It carries what decides the model's next move — the code, whether retrying
/// is safe, whether a write may have landed, and one sentence of guidance — and
/// leaves out backend detail that would only leak internals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelReport {
    /// Stable machine-readable code.
    pub code: &'static str,
    /// Caller-safe description of what went wrong.
    pub message: String,
    /// Whether repeating the identical call could succeed.
    pub retryable: bool,
    /// Suggested wait before retrying, in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    /// Whether a failed write may have been applied.
    pub side_effect: &'static str,
    /// What to do next.
    pub hint: &'static str,
}

impl Error {
    /// Render this error for a model; see [`ModelReport`].
    #[must_use]
    pub fn model_report(&self) -> ModelReport {
        let message = match self.code() {
            ErrorCode::Internal | ErrorCode::Adapter => {
                "the system could not complete the request".to_owned()
            }
            _ => self.message().to_owned(),
        };
        let side_effect = self.side_effect();
        let hint = if side_effect == SideEffectStatus::Unknown {
            "The write may have been applied. Read the record back before trying again."
        } else {
            hint_for(self.code(), self.recovery())
        };
        ModelReport {
            code: self.code().as_str(),
            message,
            retryable: self.is_retryable(),
            retry_after_ms: self
                .retry_after()
                .map(|delay| u64::try_from(delay.as_millis()).unwrap_or(u64::MAX)),
            side_effect: side_effect.as_str(),
            hint,
        }
    }
}

fn hint_for(code: ErrorCode, recovery: Option<&Recovery>) -> &'static str {
    match code {
        ErrorCode::Validation | ErrorCode::BadRequest | ErrorCode::UnknownProperty => {
            "Correct the call input to match the capability's input schema, then retry."
        }
        ErrorCode::UnknownObjectType | ErrorCode::UnknownLink | ErrorCode::UnknownAction => {
            "That name does not exist. Use tools.discover to find the right capability."
        }
        ErrorCode::NotFound => "No such record. Check the key, or search before reading.",
        ErrorCode::StaleRevision | ErrorCode::PreconditionFailed | ErrorCode::Conflict => {
            "The data changed. Read the record again, then retry with its current revision."
        }
        ErrorCode::StaleSchema => {
            "The ontology changed. Use tools.discover again before the next call."
        }
        ErrorCode::Unauthorized | ErrorCode::PolicyDenied => {
            "This is not permitted. Do not retry; tell the user what access is missing."
        }
        ErrorCode::ApprovalRequired => {
            "This needs the user's approval. Ask for it instead of retrying."
        }
        ErrorCode::UnsupportedCapability | ErrorCode::Unavailable => {
            "This cannot be done right now. Use a different capability or tell the user."
        }
        ErrorCode::BudgetExhausted => {
            "A limit was reached. Narrow the request with a filter, fewer fields or a smaller limit."
        }
        ErrorCode::Timeout => "The call ran out of time. Narrow the request, then retry once.",
        ErrorCode::Cancelled => "The call was cancelled. Do not retry unless asked.",
        ErrorCode::Transient | ErrorCode::Adapter => match recovery {
            Some(Recovery::Retry) | None => "A temporary failure. Retry once, then tell the user.",
            Some(_) => "A backend failure. Tell the user if it persists.",
        },
        _ => "The request failed. Do not retry; tell the user.",
    }
}
