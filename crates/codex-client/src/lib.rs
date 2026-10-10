// Modified from OpenAI Codex (codex-rs/codex-client/src/lib.rs @ 806d973): replaced the codex-http-client re-export with local request/error/transport modules over plain reqwest.
mod provider;
mod retry;
mod sse;
mod telemetry;

pub use crate::provider::Provider;
pub use crate::provider::RetryConfig;
pub use crate::retry::RetryOn;
pub use crate::retry::RetryOperation;
pub use crate::retry::RetryPolicy;
pub use crate::retry::backoff;
pub use crate::retry::run_with_retry;
pub use crate::sse::sse_stream;
pub use crate::telemetry::RequestTelemetry;
mod error;
mod request;
mod transport;

pub use crate::error::StreamError;
pub use crate::error::TransportError;
pub use crate::request::EncodedJsonBody;
pub use crate::request::PreparedRequestBody;
pub use crate::request::Request;
pub use crate::request::RequestBody;
pub use crate::request::RequestCompression;
pub use crate::request::Response;
pub use crate::transport::ByteStream;
pub use crate::transport::HttpTransport;
pub use crate::transport::ReqwestTransport;
pub use crate::transport::StreamResponse;
pub use codex_protocol::http_error::RetryAfter;
