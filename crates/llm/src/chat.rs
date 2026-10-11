// New in ether: an OpenAI Chat Completions client (for GLM through Zro) that yields the same `ResponseEvent`s as the Responses client.
//! Request side: the prompt's history becomes chat `messages`. Each model response (reasoning,
//! text and tool calls recorded as separate history items) is folded back into one `assistant`
//! message, replaying `reasoning_content` and tool-call argument strings exactly as received, so
//! every request starts with the previous request's bytes.
//!
//! Stream side: content and `reasoning_content` deltas stream as output text and reasoning;
//! tool-call deltas are accumulated by index and emitted as one `FunctionCall` each when the
//! stream ends; the final usage chunk becomes the `Completed` token usage.

use crate::client::ModelClient;
use crate::client::Prompt;
use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::responses::map_transport_error;
use codex_client::ByteStream;
use codex_client::EncodedJsonBody;
use codex_client::HttpTransport;
use codex_client::Provider;
use codex_client::RequestBody;
use codex_client::ReqwestTransport;
use codex_client::TransportError;
use codex_client::run_with_retry;
use codex_protocol::ResponseItemId;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::ImageReference;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use futures::future::BoxFuture;
use http::HeaderValue;
use http::Method;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::trace;
use tracing::warn;

/// Builds the Chat Completions request body for `prompt`.
pub fn build_chat_request(prompt: &Prompt) -> Value {
    let mut body = serde_json::Map::new();
    body.insert("model".to_string(), json!(prompt.model));
    body.insert("messages".to_string(), Value::Array(chat_messages(prompt)));
    let tools = chat_tools(&prompt.tools);
    if !tools.is_empty() {
        body.insert("tools".to_string(), Value::Array(tools));
    }
    if let Some(effort) = prompt
        .reasoning
        .as_ref()
        .and_then(|reasoning| reasoning.effort.as_ref())
    {
        body.insert("reasoning_effort".to_string(), json!(effort.as_str()));
    }
    if let Some(key) = &prompt.cache_key {
        body.insert("prompt_cache_key".to_string(), json!(key));
    }
    if prompt.glm_thinking {
        body.insert(
            "thinking".to_string(),
            json!({ "type": "enabled", "clear_thinking": false }),
        );
    }
    body.insert("stream".to_string(), json!(true));
    body.insert(
        "stream_options".to_string(),
        json!({ "include_usage": true }),
    );
    Value::Object(body)
}

/// Appended to an answer the server cut off at the output limit.
const TRUNCATION_NOTE: &str = "\n\n[cut off at the model's output limit]";

/// History to chat messages. Consecutive model output items (reasoning, assistant text, tool
/// calls) become one `assistant` message.
///
/// A response whose stream failed can leave its reasoning in history (the reasoning item is
/// done once the answer text starts). So a later reasoning item replaces one not yet followed
/// by text or tool calls, and an assistant message with nothing but reasoning is not sent:
/// a retried request then matches the request before it, and garbled reasoning is never
/// replayed.
fn chat_messages(prompt: &Prompt) -> Vec<Value> {
    let mut messages = Vec::new();
    if !prompt.instructions.is_empty() {
        messages.push(json!({ "role": "system", "content": prompt.instructions }));
    }
    let mut assistant: Option<AssistantMessage> = None;
    for item in &prompt.input {
        match item {
            ResponseItem::Reasoning { content, .. } => {
                let text: String = content
                    .iter()
                    .flatten()
                    .map(|part| match part {
                        ReasoningItemContent::ReasoningText { text }
                        | ReasoningItemContent::Text { text } => text.as_str(),
                    })
                    .collect();
                let message = assistant.get_or_insert_with(AssistantMessage::default);
                if message.content.is_empty() && message.tool_calls.is_empty() {
                    message.reasoning = text;
                } else {
                    message.reasoning.push_str(&text);
                }
            }
            ResponseItem::Message { role, content, .. } if role == "assistant" => {
                let text: String = content
                    .iter()
                    .filter_map(|part| match part {
                        ContentItem::OutputText { text, .. } | ContentItem::InputText { text } => {
                            Some(text.as_str())
                        }
                        _ => None,
                    })
                    .collect();
                assistant
                    .get_or_insert_with(AssistantMessage::default)
                    .content
                    .push_str(&text);
            }
            ResponseItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            } => assistant
                .get_or_insert_with(AssistantMessage::default)
                .tool_calls
                .push(tool_call(call_id, name, arguments)),
            ResponseItem::CustomToolCall {
                call_id,
                name,
                input,
                ..
            } => {
                let arguments = json!({ "input": input }).to_string();
                assistant
                    .get_or_insert_with(AssistantMessage::default)
                    .tool_calls
                    .push(tool_call(call_id, name, &arguments));
            }
            other => {
                if let Some(message) = assistant.take().filter(AssistantMessage::has_output) {
                    messages.push(message.into_json());
                }
                if let Some(message) = input_message(other) {
                    messages.push(message);
                }
            }
        }
    }
    if let Some(message) = assistant.take().filter(AssistantMessage::has_output) {
        messages.push(message.into_json());
    }
    messages
}

#[derive(Default)]
struct AssistantMessage {
    reasoning: String,
    content: String,
    tool_calls: Vec<Value>,
}

impl AssistantMessage {
    /// Whether the message has text or tool calls (reasoning alone is left out).
    fn has_output(&self) -> bool {
        !self.content.is_empty() || !self.tool_calls.is_empty()
    }
}

impl AssistantMessage {
    fn into_json(self) -> Value {
        let mut message = serde_json::Map::new();
        message.insert("role".to_string(), json!("assistant"));
        message.insert("content".to_string(), json!(self.content));
        if !self.reasoning.is_empty() {
            message.insert("reasoning_content".to_string(), json!(self.reasoning));
        }
        if !self.tool_calls.is_empty() {
            message.insert("tool_calls".to_string(), Value::Array(self.tool_calls));
        }
        Value::Object(message)
    }
}

fn tool_call(call_id: &str, name: &str, arguments: &str) -> Value {
    json!({
        "id": call_id,
        "type": "function",
        "function": { "name": name, "arguments": arguments },
    })
}

/// A non-model history item as a chat message, or `None` for items chat has no form for.
fn input_message(item: &ResponseItem) -> Option<Value> {
    match item {
        ResponseItem::Message { role, content, .. } => {
            // Zro's GLM endpoint has no `developer` role; such context goes in as `system`.
            let role = match role.as_str() {
                "developer" | "system" => "system",
                _ => "user",
            };
            Some(json!({ "role": role, "content": message_content(content) }))
        }
        ResponseItem::FunctionCallOutput {
            call_id: Some(call_id),
            output,
            ..
        }
        | ResponseItem::CustomToolCallOutput {
            call_id, output, ..
        } => Some(json!({
            "role": "tool",
            "tool_call_id": call_id,
            "content": output_text(&output.body),
        })),
        _ => None,
    }
}

/// Plain text when the message is all text, otherwise chat content parts.
fn message_content(content: &[ContentItem]) -> Value {
    let all_text = content.iter().all(|part| {
        matches!(
            part,
            ContentItem::InputText { .. } | ContentItem::OutputText { .. }
        )
    });
    if all_text {
        let text: Vec<&str> = content
            .iter()
            .filter_map(|part| match part {
                ContentItem::InputText { text } | ContentItem::OutputText { text, .. } => {
                    Some(text.as_str())
                }
                _ => None,
            })
            .collect();
        return json!(text.join("\n"));
    }
    Value::Array(
        content
            .iter()
            .filter_map(|part| match part {
                ContentItem::InputText { text } | ContentItem::OutputText { text, .. } => {
                    Some(json!({ "type": "text", "text": text }))
                }
                ContentItem::InputImage {
                    image: ImageReference::Inline { image_url },
                    ..
                } => Some(json!({ "type": "image_url", "image_url": { "url": image_url } })),
                _ => None,
            })
            .collect(),
    )
}

fn output_text(body: &FunctionCallOutputBody) -> String {
    match body {
        FunctionCallOutputBody::Text(text) => text.clone(),
        FunctionCallOutputBody::ContentItems(items) => items
            .iter()
            .filter_map(|item| match item {
                FunctionCallOutputContentItem::InputText { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// Responses tool definitions to chat function tools. Only function tools have a chat form.
fn chat_tools(tools: &[Value]) -> Vec<Value> {
    tools
        .iter()
        .filter(|tool| tool.get("type").and_then(Value::as_str) == Some("function"))
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": tool.get("name").cloned().unwrap_or(Value::Null),
                    "description": tool.get("description").cloned().unwrap_or(Value::Null),
                    "parameters": tool.get("parameters").cloned().unwrap_or_else(|| json!({})),
                },
            })
        })
        .collect()
}

/// [`ModelClient`] over Chat Completions (`POST /chat/completions`).
pub struct ChatCompletionsModelClient {
    transport: ReqwestTransport,
    provider: Provider,
    api_key: Option<String>,
}

impl ChatCompletionsModelClient {
    pub fn new(transport: ReqwestTransport, provider: Provider, api_key: Option<String>) -> Self {
        Self {
            transport,
            provider,
            api_key,
        }
    }

    async fn stream_prompt(&self, prompt: &Prompt) -> Result<ResponseStream, ApiError> {
        let body = EncodedJsonBody::encode(&build_chat_request(prompt))
            .map_err(|e| ApiError::Stream(format!("failed to encode chat request: {e}")))?;
        let mut request = self
            .provider
            .build_request(Method::POST, "/chat/completions");
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
        let response = run_with_retry(
            self.provider.retry.to_policy(),
            || request.clone(),
            |req, _attempt| self.transport.stream(req),
        )
        .await
        .map_err(map_transport_error)?;
        let upstream_request_id = response
            .headers
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let (tx_event, rx_event) = mpsc::channel(1600);
        tokio::spawn(process_chat_sse(
            response.bytes,
            tx_event,
            self.provider.stream_idle_timeout,
        ));
        Ok(ResponseStream {
            rx_event,
            upstream_request_id,
        })
    }
}

impl ModelClient for ChatCompletionsModelClient {
    fn stream<'a>(&'a self, prompt: &'a Prompt) -> BoxFuture<'a, Result<ResponseStream, ApiError>> {
        Box::pin(self.stream_prompt(prompt))
    }
}

#[derive(Debug, Deserialize)]
struct ChatChunk {
    #[serde(default, deserialize_with = "null_as_default")]
    id: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    choices: Vec<ChatChoice>,
    #[serde(default, deserialize_with = "null_as_default")]
    usage: Option<ChatUsage>,
    #[serde(default, deserialize_with = "null_as_default")]
    error: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    #[serde(default, deserialize_with = "null_as_default")]
    delta: Option<ChatDelta>,
    #[serde(default, deserialize_with = "null_as_default")]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatDelta {
    #[serde(default, deserialize_with = "null_as_default")]
    content: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    reasoning_content: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    tool_calls: Vec<ToolCallDelta>,
}

#[derive(Debug, Deserialize)]
struct ToolCallDelta {
    #[serde(default, deserialize_with = "null_as_default")]
    index: Option<usize>,
    #[serde(default, deserialize_with = "null_as_default")]
    id: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    function: Option<FunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct FunctionDelta {
    #[serde(default, deserialize_with = "null_as_default")]
    name: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatUsage {
    #[serde(default, deserialize_with = "null_as_default")]
    prompt_tokens: i64,
    #[serde(default, deserialize_with = "null_as_default")]
    completion_tokens: i64,
    #[serde(default, deserialize_with = "null_as_default")]
    total_tokens: i64,
    #[serde(default, deserialize_with = "null_as_default")]
    prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(default, deserialize_with = "null_as_default")]
    completion_tokens_details: Option<CompletionTokensDetails>,
}

#[derive(Debug, Deserialize)]
struct PromptTokensDetails {
    #[serde(default, deserialize_with = "null_as_default")]
    cached_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct CompletionTokensDetails {
    #[serde(default, deserialize_with = "null_as_default")]
    reasoning_tokens: i64,
}

/// Reads `null` like a missing field: some servers send `"tool_calls": null` or
/// `"cached_tokens": null`, which would otherwise fail the whole chunk.
fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

impl From<ChatUsage> for TokenUsage {
    fn from(usage: ChatUsage) -> Self {
        TokenUsage {
            input_tokens: usage.prompt_tokens,
            cached_input_tokens: usage
                .prompt_tokens_details
                .map(|details| details.cached_tokens)
                .unwrap_or(0),
            cache_write_input_tokens: 0,
            output_tokens: usage.completion_tokens,
            reasoning_output_tokens: usage
                .completion_tokens_details
                .map(|details| details.reasoning_tokens)
                .unwrap_or(0),
            total_tokens: usage.total_tokens,
            codex_rollout_budget_units: None,
        }
    }
}

#[derive(Default)]
struct PendingCall {
    id: String,
    name: String,
    arguments: String,
}

/// Turns one chat stream into `ResponseEvent`s.
#[derive(Default)]
pub(crate) struct ChatStreamState {
    response_id: Option<String>,
    created: bool,
    /// Id and text of the reasoning item being streamed.
    reasoning: Option<(String, String)>,
    /// Id and text of the assistant message being streamed.
    text: Option<(String, String)>,
    calls: BTreeMap<usize, PendingCall>,
    usage: Option<TokenUsage>,
    item_counter: usize,
    /// The `finish_reason` the server sent, if any.
    finish_reason: Option<String>,
}

impl ChatStreamState {
    fn next_id(&mut self, prefix: &str) -> String {
        self.item_counter += 1;
        let response = self.response_id.as_deref().unwrap_or("chat");
        format!("{prefix}_{response}_{}", self.item_counter)
    }

    /// Events for one parsed chunk.
    fn on_chunk(&mut self, chunk: ChatChunk) -> Result<Vec<ResponseEvent>, ApiError> {
        if let Some(error) = chunk.error {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| error.to_string());
            if crate::overflow::is_overflow(&message) {
                return Err(ApiError::ContextWindowExceeded);
            }
            return Err(ApiError::Stream(message));
        }
        let mut events = Vec::new();
        if self.response_id.is_none() {
            self.response_id = chunk.id.clone();
        }
        if !self.created {
            self.created = true;
            events.push(ResponseEvent::Created {
                response_id: self.response_id.clone(),
            });
        }
        if let Some(usage) = chunk.usage {
            self.usage = Some(usage.into());
        }
        for choice in chunk.choices {
            let Some(delta) = choice.delta else {
                continue;
            };
            if let Some(reasoning) = delta.reasoning_content.filter(|text| !text.is_empty()) {
                if self.reasoning.is_none() {
                    let id = self.next_id("rs");
                    events.push(ResponseEvent::OutputItemAdded(reasoning_item(&id, "")));
                    self.reasoning = Some((id, String::new()));
                }
                if let Some((_, buffer)) = self.reasoning.as_mut() {
                    buffer.push_str(&reasoning);
                }
                events.push(ResponseEvent::ReasoningContentDelta {
                    delta: reasoning,
                    content_index: 0,
                });
            }
            if let Some(text) = delta.content.filter(|text| !text.is_empty()) {
                // Reasoning precedes the answer; close it when text starts.
                events.extend(self.finish_reasoning());
                if self.text.is_none() {
                    let id = self.next_id("msg");
                    events.push(ResponseEvent::OutputItemAdded(message_item(&id, "")));
                    self.text = Some((id, String::new()));
                }
                if let Some((_, buffer)) = self.text.as_mut() {
                    buffer.push_str(&text);
                }
                events.push(ResponseEvent::OutputTextDelta(text));
            }
            for call in delta.tool_calls {
                let index = call.index.unwrap_or(self.calls.len());
                let pending = self.calls.entry(index).or_default();
                if let Some(id) = call.id {
                    pending.id = id;
                }
                if let Some(function) = call.function {
                    // Some servers repeat the whole name on every delta.
                    if let Some(name) = function.name
                        && name != pending.name
                    {
                        pending.name.push_str(&name);
                    }
                    if let Some(arguments) = function.arguments {
                        pending.arguments.push_str(&arguments);
                    }
                }
            }
            if let Some(reason) = choice.finish_reason {
                trace!("chat finish_reason: {reason}");
                if reason == "content_filter" {
                    return Err(ApiError::ContentFilter);
                }
                // Z.AI reports an oversized request as a finish reason.
                if crate::overflow::is_overflow(&reason) {
                    return Err(ApiError::ContextWindowExceeded);
                }
                self.finish_reason = Some(reason);
            }
        }
        Ok(events)
    }

    fn finish_reasoning(&mut self) -> Vec<ResponseEvent> {
        match self.reasoning.take() {
            Some((id, text)) => vec![ResponseEvent::OutputItemDone(reasoning_item(&id, &text))],
            None => Vec::new(),
        }
    }

    /// Events that close the response: open items, tool calls in index order, completion.
    /// A response cut off at the output limit (`finish_reason: length`) is an error when it was
    /// writing a tool call (its arguments are incomplete); a cut-off answer is kept with a note.
    fn finish(&mut self) -> Result<Vec<ResponseEvent>, ApiError> {
        let truncated = self.finish_reason.as_deref() == Some("length");
        // Cut off before writing anything: the input filled the window (a silent overflow).
        if truncated && self.text.is_none() && self.calls.is_empty() {
            return Err(ApiError::ContextWindowExceeded);
        }
        if truncated && !self.calls.is_empty() {
            return Err(ApiError::OutputTruncated);
        }
        let mut events = self.finish_reasoning();
        if truncated && let Some((_, text)) = self.text.as_mut() {
            text.push_str(TRUNCATION_NOTE);
            events.push(ResponseEvent::OutputTextDelta(TRUNCATION_NOTE.to_string()));
        }
        if let Some((id, text)) = self.text.take() {
            events.push(ResponseEvent::OutputItemDone(message_item(&id, &text)));
        }
        for (index, call) in std::mem::take(&mut self.calls) {
            let call_id = if call.id.is_empty() {
                format!("call_{index}")
            } else {
                call.id
            };
            events.push(ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
                encrypted_content: None,
                status: None,
                id: None,
                name: call.name,
                namespace: None,
                arguments: call.arguments,
                encrypted_function_args: None,
                call_id,
                internal_chat_message_metadata_passthrough: None,
            }));
        }
        events.push(ResponseEvent::Completed {
            response_id: self.response_id.clone().unwrap_or_default(),
            token_usage: self.usage.take(),
            usage_metadata: None,
            end_turn: None,
        });
        Ok(events)
    }
}

fn reasoning_item(id: &str, text: &str) -> ResponseItem {
    ResponseItem::Reasoning {
        id: Some(ResponseItemId::from_server(id.to_string())),
        summary: Vec::new(),
        content: Some(vec![ReasoningItemContent::ReasoningText {
            text: text.to_string(),
        }]),
        encrypted_content: None,
        internal_chat_message_metadata_passthrough: None,
    }
}

fn message_item(id: &str, text: &str) -> ResponseItem {
    ResponseItem::Message {
        encrypted_content: None,
        status: None,
        id: Some(ResponseItemId::from_server(id.to_string())),
        role: "assistant".to_string(),
        content: if text.is_empty() {
            Vec::new()
        } else {
            vec![ContentItem::OutputText {
                text: text.to_string(),
                annotations: None,
                logprobs: None,
            }]
        },
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }
}

async fn process_chat_sse(
    stream: ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
) {
    let mut stream = stream.eventsource();
    let mut state = ChatStreamState::default();
    loop {
        let next = tokio::select! {
            biased;
            _ = tx_event.closed() => return,
            next = timeout(idle_timeout, stream.next()) => next,
        };
        let sse = match next {
            Ok(Some(Ok(sse))) => sse,
            Ok(Some(Err(error))) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream(error.to_string())))
                    .await;
                return;
            }
            // Some servers end the stream without `[DONE]`: a clean end is a completion if the
            // server said why it finished; otherwise the response is incomplete.
            Ok(None) => {
                if state.finish_reason.is_none() {
                    let _ = tx_event
                        .send(Err(ApiError::Stream(
                            "chat stream ended before the response finished".to_string(),
                        )))
                        .await;
                    return;
                }
                break;
            }
            Err(_) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream("idle timeout waiting for SSE".into())))
                    .await;
                return;
            }
        };
        trace!("chat SSE event: {}", &sse.data);
        if sse.data.trim() == "[DONE]" {
            break;
        }
        let chunk: ChatChunk = match serde_json::from_str(&sse.data) {
            Ok(chunk) => chunk,
            Err(error) => {
                warn!("skipping a chat chunk that does not parse: {error}");
                continue;
            }
        };
        match state.on_chunk(chunk) {
            Ok(events) => {
                for event in events {
                    if tx_event.send(Ok(event)).await.is_err() {
                        return;
                    }
                }
            }
            Err(error) => {
                let _ = tx_event.send(Err(error)).await;
                return;
            }
        }
    }
    if !state.created {
        let _ = tx_event
            .send(Err(ApiError::Stream(
                "chat stream ended without any data".to_string(),
            )))
            .await;
        return;
    }
    match state.finish() {
        Ok(events) => {
            for event in events {
                if tx_event.send(Ok(event)).await.is_err() {
                    return;
                }
            }
        }
        Err(error) => {
            let _ = tx_event.send(Err(error)).await;
        }
    }
}
