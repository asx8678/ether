// Modified from OpenAI Codex (codex-rs/codex-api/src/sse/responses.rs @ 806d973): removed unit tests; removed rate-limit, models-etag, server-model, reasoning-included, turn-state, safety-buffering, model-verification, moderation-metadata and telemetry handling.
use super::responses_error::parse_failed_response;
use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::error::parse_flex_unavailable;
use codex_client::ByteStream;
use codex_client::StreamResponse;
use codex_protocol::ResponseUsageMetadata;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::debug;
use tracing::trace;

const REQUEST_ID_HEADER: &str = "x-request-id";

pub fn spawn_response_stream(
    stream_response: StreamResponse,
    idle_timeout: Duration,
) -> ResponseStream {
    let upstream_request_id = stream_response
        .headers
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(process_sse(stream_response.bytes, tx_event, idle_timeout));

    ResponseStream {
        rx_event,
        upstream_request_id,
    }
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ResponseCompleted {
    id: String,
    #[serde(default)]
    usage: Option<ResponseCompletedUsage>,
    usage_metadata: Option<ResponseUsageMetadata>,
    #[serde(default)]
    end_turn: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ResponseCompletedUsage {
    input_tokens: i64,
    input_tokens_details: Option<ResponseCompletedInputTokensDetails>,
    output_tokens: i64,
    output_tokens_details: Option<ResponseCompletedOutputTokensDetails>,
    total_tokens: i64,
    #[serde(default)]
    codex_rollout_budget_units: Option<serde_json::Number>,
}

impl From<ResponseCompletedUsage> for TokenUsage {
    fn from(val: ResponseCompletedUsage) -> Self {
        let input_tokens_details = val.input_tokens_details.unwrap_or_default();
        TokenUsage {
            input_tokens: val.input_tokens,
            cached_input_tokens: input_tokens_details.cached_tokens,
            cache_write_input_tokens: input_tokens_details.cache_write_tokens,
            output_tokens: val.output_tokens,
            reasoning_output_tokens: val
                .output_tokens_details
                .map(|d| d.reasoning_tokens)
                .unwrap_or(0),
            total_tokens: val.total_tokens,
            codex_rollout_budget_units: val.codex_rollout_budget_units,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct ResponseCompletedInputTokensDetails {
    cached_tokens: i64,
    #[serde(default)]
    cache_write_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct ResponseCompletedOutputTokensDetails {
    reasoning_tokens: i64,
}

#[derive(Deserialize, Debug)]
pub struct ResponsesStreamEvent {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    response: Option<Value>,
    error: Option<Value>,
    item: Option<Value>,
    item_id: Option<String>,
    call_id: Option<String>,
    delta: Option<String>,
    text: Option<String>,
    summary_index: Option<i64>,
    content_index: Option<i64>,
}

pub fn process_responses_event(
    event: ResponsesStreamEvent,
) -> std::result::Result<Option<ResponseEvent>, ApiError> {
    match event.kind.as_str() {
        "error" => {
            if let Some(error) = event.error.as_ref().and_then(parse_flex_unavailable) {
                return Err(error);
            }
        }
        "response.output_item.done" => {
            if let Some(item_val) = event.item {
                if let Ok(item) = serde_json::from_value::<ResponseItem>(item_val) {
                    return Ok(Some(ResponseEvent::OutputItemDone(item)));
                }
                debug!("failed to parse ResponseItem from output_item.done");
            }
        }
        "response.output_text.delta" => {
            if let Some(delta) = event.delta {
                return Ok(Some(ResponseEvent::OutputTextDelta(delta)));
            }
        }
        "response.custom_tool_call_input.delta" => {
            if let (Some(delta), Some(item_id)) =
                (event.delta, event.item_id.clone().or(event.call_id.clone()))
            {
                return Ok(Some(ResponseEvent::ToolCallInputDelta {
                    item_id,
                    call_id: event.call_id,
                    delta,
                }));
            }
        }
        "response.reasoning_summary_text.delta" => {
            if let (Some(delta), Some(summary_index)) = (event.delta, event.summary_index) {
                return Ok(Some(ResponseEvent::ReasoningSummaryDelta {
                    delta,
                    summary_index,
                }));
            }
        }
        "response.reasoning_summary_text.done" => {
            if let (Some(item_id), Some(text), Some(summary_index)) =
                (event.item_id, event.text, event.summary_index)
            {
                return Ok(Some(ResponseEvent::ReasoningSummaryDone {
                    item_id,
                    text,
                    summary_index,
                }));
            }
        }
        "response.reasoning_text.delta" => {
            if let (Some(delta), Some(content_index)) = (event.delta, event.content_index) {
                return Ok(Some(ResponseEvent::ReasoningContentDelta {
                    delta,
                    content_index,
                }));
            }
        }
        "response.created" => {
            if let Some(response) = event.response {
                let response_id = response
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                return Ok(Some(ResponseEvent::Created { response_id }));
            }
        }
        "response.failed" => {
            return Err(parse_failed_response(event.response));
        }
        "response.completed" | "response.incomplete" => {
            let interrupted = event.kind == "response.incomplete";
            if interrupted {
                let reason = event.response.as_ref().and_then(|response| {
                    response
                        .get("incomplete_details")
                        .and_then(|details| details.get("reason"))
                        .and_then(Value::as_str)
                });
                let reason = reason.unwrap_or("unknown");
                if reason == "content_filter" {
                    return Err(ApiError::ContentFilter);
                }
                if reason != "interrupted" {
                    let message = format!("Incomplete response returned, reason: {reason}");
                    return Err(ApiError::Stream(message));
                }
            }
            if let Some(resp_val) = event.response {
                let metadata = resp_val
                    .get("usage")
                    .filter(|usage| !usage.is_null())
                    .cloned();
                match serde_json::from_value::<ResponseCompleted>(resp_val) {
                    Ok(mut resp) => {
                        if let Some(metadata) = metadata {
                            resp.usage_metadata.get_or_insert_default().metadata = Some(metadata);
                        }
                        return Ok(Some(ResponseEvent::Completed {
                            response_id: resp.id,
                            token_usage: resp.usage.map(Into::into),
                            usage_metadata: resp.usage_metadata,
                            end_turn: if interrupted {
                                Some(false)
                            } else {
                                resp.end_turn
                            },
                        }));
                    }
                    Err(err) => {
                        let error = format!("failed to parse ResponseCompleted: {err}");
                        debug!("{error}");
                        return Err(ApiError::Stream(error));
                    }
                }
            }
        }
        "response.output_item.added" => {
            if let Some(item_val) = event.item {
                if let Ok(item) = serde_json::from_value::<ResponseItem>(item_val) {
                    return Ok(Some(ResponseEvent::OutputItemAdded(item)));
                }
                debug!("failed to parse ResponseItem from output_item.added");
            }
        }
        "response.reasoning_summary_part.added" => {
            if let Some(summary_index) = event.summary_index {
                return Ok(Some(ResponseEvent::ReasoningSummaryPartAdded {
                    summary_index,
                }));
            }
        }
        "codex.response.metadata"
        | "response.content_part.added"
        | "response.content_part.done"
        | "response.custom_tool_call_input.done"
        | "response.function_call_arguments.delta"
        | "response.function_call_arguments.done"
        | "response.in_progress"
        | "response.metadata"
        | "response.output_text.done"
        | "response.reasoning_summary_part.done" => {
            trace!("unhandled responses event: {}", event.kind);
        }
        kind if kind.ends_with(".delta") => {
            trace!("unhandled responses event: {kind}");
        }
        _ => {
            debug!(
                "unhandled responses event: {:?}",
                event.kind.chars().take(128).collect::<String>()
            );
        }
    }

    Ok(None)
}

pub(crate) async fn process_sse(
    stream: ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
) {
    let mut stream = stream.eventsource();
    let mut response_error: Option<ApiError> = None;

    loop {
        let response = tokio::select! {
            biased;
            _ = tx_event.closed() => return,
            response = timeout(idle_timeout, stream.next()) => response,
        };
        let sse = match response {
            Ok(Some(Ok(sse))) => sse,
            Ok(Some(Err(e))) => {
                debug!("SSE Error: {e:#}");
                let _ = tx_event.send(Err(ApiError::Stream(e.to_string()))).await;
                return;
            }
            Ok(None) => {
                let error = response_error.unwrap_or(ApiError::Stream(
                    "stream closed before response.completed".into(),
                ));
                let _ = tx_event.send(Err(error)).await;
                return;
            }
            Err(_) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream("idle timeout waiting for SSE".into())))
                    .await;
                return;
            }
        };

        trace!("SSE event: {}", &sse.data);

        let event: ResponsesStreamEvent = match serde_json::from_str(&sse.data) {
            Ok(event) => event,
            Err(e) => {
                debug!(
                    error_category = ?e.classify(),
                    error_line = e.line(),
                    error_column = e.column(),
                    payload_bytes = sse.data.len(),
                    "Failed to parse SSE event"
                );
                continue;
            }
        };

        match process_responses_event(event) {
            Ok(Some(event)) => {
                let is_completed = matches!(event, ResponseEvent::Completed { .. });
                if tx_event.send(Ok(event)).await.is_err() {
                    return;
                }
                if is_completed {
                    return;
                }
            }
            Ok(None) => {}
            Err(error) => {
                if matches!(error, ApiError::FlexUnavailable) {
                    let _ = tx_event.send(Err(error)).await;
                    return;
                }
                response_error = Some(error);
            }
        };
    }
}
