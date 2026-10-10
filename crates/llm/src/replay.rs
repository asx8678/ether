// New in ether: a model client that replays recorded Responses API events from a file.
//! Lets the binary run end to end without a network model. Each event goes through the same
//! SSE parser as a live response, so a replayed turn exercises the real stream handling.
//!
//! File format: one Responses stream event per line, as the JSON object a server sends in an
//! SSE `data:` field (a leading `data: ` is accepted and stripped). Blank lines and lines
//! starting with `#` are ignored. A `response.completed`, `response.incomplete` or
//! `response.failed` event ends one response; the next model request gets the next response.

use crate::client::ModelClient;
use crate::client::Prompt;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::sse::spawn_response_stream;
use bytes::Bytes;
use codex_client::StreamResponse;
use futures::future::BoxFuture;
use http::HeaderMap;
use http::StatusCode;
use std::collections::VecDeque;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

const TERMINAL_EVENTS: [&str; 3] = [
    "response.completed",
    "response.incomplete",
    "response.failed",
];

/// Errors reading a replay file.
#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("cannot read replay file {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path}:{line}: not a JSON event: {source}")]
    Parse {
        path: PathBuf,
        line: usize,
        source: serde_json::Error,
    },
    #[error("{path}: no responses found")]
    Empty { path: PathBuf },
}

/// [`ModelClient`] that answers each request with the next recorded response.
pub struct ReplayModelClient {
    responses: Mutex<VecDeque<Bytes>>,
    request_log: Option<Mutex<std::fs::File>>,
}

impl ReplayModelClient {
    /// Loads every response in `path`. Fails if the file has a malformed line.
    pub fn from_file(path: &Path) -> Result<Self, ReplayError> {
        let text = std::fs::read_to_string(path).map_err(|source| ReplayError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let responses = split_responses(&text, path)?;
        if responses.is_empty() {
            return Err(ReplayError::Empty {
                path: path.to_path_buf(),
            });
        }
        Ok(Self {
            responses: Mutex::new(responses),
            request_log: None,
        })
    }

    /// Appends each request's prompt as one JSON line to `path`.
    pub fn with_request_log(mut self, path: &Path) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        self.request_log = Some(Mutex::new(file));
        Ok(self)
    }

    fn log_request(&self, prompt: &Prompt) {
        let Some(log) = self.request_log.as_ref() else {
            return;
        };
        let Ok(mut line) = serde_json::to_vec(prompt) else {
            return;
        };
        line.push(b'\n');
        if let Ok(mut file) = log.lock() {
            let _ = file.write_all(&line);
        }
    }

    fn next_response(&self) -> Option<Bytes> {
        self.responses.lock().ok()?.pop_front()
    }
}

impl ModelClient for ReplayModelClient {
    fn stream<'a>(&'a self, prompt: &'a Prompt) -> BoxFuture<'a, Result<ResponseStream, ApiError>> {
        Box::pin(async move {
            self.log_request(prompt);
            let Some(body) = self.next_response() else {
                return Err(ApiError::InvalidRequest {
                    message: "replay file has no more recorded responses".to_string(),
                });
            };
            let bytes = futures::stream::once(async move { Ok(body) });
            let response = StreamResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                bytes: Box::pin(bytes),
            };
            Ok(spawn_response_stream(response, Duration::from_secs(5)))
        })
    }
}

/// Groups the file's events into SSE bodies, one per recorded response.
fn split_responses(text: &str, path: &Path) -> Result<VecDeque<Bytes>, ReplayError> {
    let mut responses = VecDeque::new();
    let mut current = String::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let data = line.strip_prefix("data:").map_or(line, str::trim_start);
        let event: serde_json::Value =
            serde_json::from_str(data).map_err(|source| ReplayError::Parse {
                path: path.to_path_buf(),
                line: index + 1,
                source,
            })?;
        current.push_str("data: ");
        current.push_str(data);
        current.push_str("\n\n");
        let kind = event.get("type").and_then(serde_json::Value::as_str);
        if kind.is_some_and(|kind| TERMINAL_EVENTS.contains(&kind)) {
            responses.push_back(Bytes::from(std::mem::take(&mut current)));
        }
    }
    if !current.is_empty() {
        // A trailing response without a terminal event replays as a stream that closes early.
        responses.push_back(Bytes::from(current));
    }
    Ok(responses)
}
