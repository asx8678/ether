// Modified from OpenAI Codex (codex-rs/codex-api/src/error.rs @ 806d973): `RetryAfter` comes from codex-protocol; the rate-limit header error is removed; `OutputTruncated` added (ether).
use codex_client::TransportError;
use codex_protocol::http_error::RetryAfter;
use codex_protocol::protocol::MisalignmentErrorDetails;
use http::StatusCode;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error("api error {status}: {message}")]
    Api { status: StatusCode, message: String },
    #[error("stream error: {0}")]
    Stream(String),
    #[error("stream error: Incomplete response returned, reason: content_filter")]
    ContentFilter,
    #[error("context window exceeded")]
    ContextWindowExceeded,
    /// The response was cut off at the model's output limit while writing a tool call (ether).
    #[error("the response was cut off at the model's output limit while writing a tool call")]
    OutputTruncated,
    #[error("quota exceeded")]
    QuotaExceeded,
    #[error("usage not included")]
    UsageNotIncluded,
    #[error("retryable error: {message}")]
    Retryable {
        message: String,
        retry_after: Option<RetryAfter>,
    },
    #[error("rate limit exceeded: {message}")]
    RateLimitExceeded {
        message: String,
        retry_after: Option<RetryAfter>,
    },
    #[error("invalid request: {message}")]
    InvalidRequest { message: String },
    #[error("invalid prompt: {message}")]
    InvalidPrompt { message: String },
    #[error("cyber policy: {message}")]
    CyberPolicy { message: String },
    #[error("bio policy: {message}")]
    BioPolicy { message: String },
    #[error("misalignment policy violation: {message}")]
    MisalignmentPolicyViolation {
        message: String,
        misalignment: Option<MisalignmentErrorDetails>,
    },
    #[error("Flex capacity unavailable.")]
    FlexUnavailable,
    #[error("server overloaded")]
    ServerOverloaded { retry_after: Option<RetryAfter> },
}

pub(crate) fn parse_flex_unavailable(error: &Value) -> Option<ApiError> {
    (error.get("code").and_then(Value::as_str) == Some("flex_unavailable"))
        .then_some(ApiError::FlexUnavailable)
}
