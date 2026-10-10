// `RetryAfter` is copied from OpenAI Codex (codex-rs/http-client/src/retry_after.rs @ 806d973).
// `HttpError` is new: it replaces codex-http-client's `RouteAwareRequestError` with a
// transport-neutral error that model clients build from their own HTTP errors.
//! HTTP error types shared by protocol errors and model clients.

use http::HeaderMap;
use http::StatusCode;
use http::header::RETRY_AFTER;
use std::time::Duration;
use std::time::SystemTime;
use tokio::time::Instant;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryAfter(Instant);

impl RetryAfter {
    pub fn from_delay(delay: Duration) -> Option<Self> {
        Instant::now().checked_add(delay).map(Self)
    }

    pub fn from_headers(headers: &HeaderMap) -> Option<Self> {
        Self::from_header(headers.get(RETRY_AFTER)?.to_str().ok()?)
    }

    pub fn from_header(value: &str) -> Option<Self> {
        let now = SystemTime::now();
        let received_at = Instant::now();
        let value = value.trim();
        let delay = if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
            Duration::from_secs(value.parse().ok()?)
        } else {
            httpdate::parse_http_date(value)
                .ok()?
                .duration_since(now)
                .unwrap_or_default()
        };
        received_at.checked_add(delay).map(Self)
    }

    pub fn deadline(self) -> Instant {
        self.0
    }

    pub fn remaining_delay(self) -> Duration {
        self.0.saturating_duration_since(Instant::now())
    }
}

/// A failed HTTP exchange: transport failure or a non-success response.
#[derive(Debug)]
pub struct HttpError {
    message: String,
    status: Option<StatusCode>,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl HttpError {
    pub fn new(message: impl Into<String>, status: Option<StatusCode>) -> Self {
        Self {
            message: message.into(),
            status,
            source: None,
        }
    }

    pub fn from_source(
        source: impl std::error::Error + Send + Sync + 'static,
        status: Option<StatusCode>,
    ) -> Self {
        Self {
            message: source.to_string(),
            status,
            source: Some(Box::new(source)),
        }
    }

    pub fn status(&self) -> Option<StatusCode> {
        self.status
    }
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HttpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}
