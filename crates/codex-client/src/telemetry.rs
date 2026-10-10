// Modified from OpenAI Codex (codex-rs/codex-client/src/telemetry.rs @ 806d973): uses local types instead of codex-http-client.
use crate::TransportError;
use http::StatusCode;
use std::time::Duration;

/// API specific telemetry.
pub trait RequestTelemetry: Send + Sync {
    fn on_request(
        &self,
        attempt: u64,
        status: Option<StatusCode>,
        error: Option<&TransportError>,
        duration: Duration,
    );
}
