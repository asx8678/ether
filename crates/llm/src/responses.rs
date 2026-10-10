// Modified from OpenAI Codex (codex-rs/codex-api/src/endpoint/responses.rs and endpoint/session.rs @ 806d973): one HTTP client for POST /responses with bearer API-key auth; WebSocket, session/thread/subagent headers, request compression, turn state and telemetry are removed.
use crate::common::ResponseStream;
use crate::common::ResponsesApiRequest;
use crate::error::ApiError;
use crate::sse::spawn_response_stream;
use codex_client::EncodedJsonBody;
use codex_client::HttpTransport;
use codex_client::Provider;
use codex_client::RequestBody;
use codex_client::ReqwestTransport;
use codex_client::TransportError;
use codex_client::run_with_retry;
use http::HeaderValue;
use http::Method;

/// Streams Responses API turns from one provider endpoint.
pub struct ResponsesClient {
    transport: ReqwestTransport,
    provider: Provider,
    api_key: Option<String>,
}

impl ResponsesClient {
    pub fn new(transport: ReqwestTransport, provider: Provider, api_key: Option<String>) -> Self {
        Self {
            transport,
            provider,
            api_key,
        }
    }

    pub async fn stream_request(
        &self,
        request: &ResponsesApiRequest,
    ) -> Result<ResponseStream, ApiError> {
        let body = EncodedJsonBody::encode(request)
            .map_err(|e| ApiError::Stream(format!("failed to encode responses request: {e}")))?;
        let mut request = self.provider.build_request(Method::POST, "/responses");
        request.body = Some(RequestBody::EncodedJson(body));
        request.headers.insert(
            http::header::ACCEPT,
            HeaderValue::from_static("text/event-stream"),
        );
        if let Some(api_key) = self.api_key.as_deref() {
            let value = HeaderValue::from_str(&format!("Bearer {api_key}"))
                .map_err(|_| TransportError::Build("API key is not a valid header value".into()))?;
            request.headers.insert(http::header::AUTHORIZATION, value);
        }
        let request = request.into_prepared().map_err(TransportError::Build)?;

        let stream_response = run_with_retry(
            self.provider.retry.to_policy(),
            || request.clone(),
            |req, _attempt| self.transport.stream(req),
        )
        .await
        .map_err(map_transport_error)?;

        Ok(spawn_response_stream(
            stream_response,
            self.provider.stream_idle_timeout,
        ))
    }
}

/// Turns an HTTP error status into [`ApiError::Api`] with the server's error message.
fn map_transport_error(error: TransportError) -> ApiError {
    match error {
        TransportError::Http { status, body, .. } => {
            let message = body
                .as_deref()
                .and_then(|body| serde_json::from_str::<serde_json::Value>(body).ok())
                .and_then(|value| {
                    value
                        .get("error")
                        .and_then(|error| error.get("message"))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                })
                .or(body)
                .unwrap_or_default();
            ApiError::Api { status, message }
        }
        error => ApiError::Transport(error),
    }
}
