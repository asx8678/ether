// New in ether: the model-client interface the engine calls, and its Responses API implementation.
//! A [`Prompt`] is everything one model request needs; a [`ModelClient`] turns it into a stream
//! of [`ResponseEvent`]s. The engine only sees this interface, so the wire format underneath
//! (Responses now, Chat Completions in Phase 4, or a replay file) can change without it knowing.

use crate::chat::ChatCompletionsModelClient;
use crate::chat::build_chat_request;
use crate::common::Reasoning;
use crate::common::ResponseStream;
use crate::common::ResponsesApiRequest;
use crate::common::create_text_param_for_request;
use crate::error::ApiError;
use crate::responses::ResponsesClient;
use codex_client::ReqwestTransport;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::WireApi;
use codex_protocol::config_types::Verbosity;
use codex_protocol::models::ResponseItem;
use futures::future::BoxFuture;
use serde::Serialize;
use serde_json::Value;
use std::io::Write;
use std::path::Path;
use codex_utils_redacted_string::RedactedString;
use std::sync::Arc;
use std::sync::Mutex;

/// One model request.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Prompt {
    /// Model slug sent to the provider.
    pub model: String,
    /// Base instructions (the system prompt). Must be byte-identical across requests so
    /// providers that cache by prefix can reuse it.
    pub instructions: String,
    /// Conversation history, oldest first.
    pub input: Vec<ResponseItem>,
    /// Tool definitions in Responses API JSON form, in a stable order.
    pub tools: Vec<Value>,
    /// Whether the model may call several tools in one response.
    pub parallel_tool_calls: bool,
    /// Reasoning settings; `None` for models without reasoning controls.
    pub reasoning: Option<Reasoning>,
    /// Output verbosity, for models that support it.
    pub verbosity: Option<Verbosity>,
    /// Optional JSON schema for the final answer.
    pub output_schema: Option<Value>,
    /// Chat Completions only: send Z.AI's `thinking: {type: "enabled", clear_thinking: false}`,
    /// which keeps GLM's reasoning across turns (ether replays it in each request).
    pub glm_thinking: bool,
}

/// Streams model responses for prompts. Implementations hold no per-turn state.
pub trait ModelClient: Send + Sync {
    fn stream<'a>(&'a self, prompt: &'a Prompt) -> BoxFuture<'a, Result<ResponseStream, ApiError>>;
}

/// Errors building a model client from provider configuration.
#[derive(Debug, thiserror::Error)]
pub enum ClientBuildError {
    #[error("provider `{name}`: {message}")]
    Provider { name: String, message: String },
}

/// Builds the client for `info`'s wire API: Responses or Chat Completions.
pub fn client_for_provider(
    info: &ModelProviderInfo,
) -> Result<Arc<dyn ModelClient>, ClientBuildError> {
    let provider_error = |err: codex_protocol::error::CodexErr| ClientBuildError::Provider {
        name: info.name.clone(),
        message: err.to_string(),
    };
    let unsupported = |message: &str| ClientBuildError::Provider {
        name: info.name.clone(),
        message: message.to_string(),
    };
    info.validate().map_err(|message| unsupported(&message))?;
    if info.auth.is_some() || info.gateway_oauth.is_some() {
        return Err(unsupported(
            "`auth` commands and `gateway_oauth` are not supported by ether; use `env_key` or \
             `experimental_bearer_token`",
        ));
    }
    let provider = info.to_api_provider(None).map_err(provider_error)?;
    let api_key = match info.api_key().map_err(provider_error)? {
        Some(key) => Some(key),
        None => info
            .experimental_bearer_token
            .clone()
            .map(RedactedString::into_inner),
    };
    let transport = ReqwestTransport::new(reqwest::Client::new());
    Ok(match info.wire_api {
        WireApi::Responses => Arc::new(ResponsesModelClient {
            client: ResponsesClient::new(transport, provider, api_key),
        }),
        WireApi::ChatCompletions => Arc::new(ChatCompletionsModelClient::new(
            transport, provider, api_key,
        )),
    })
}

/// The request body a prompt becomes on the given wire API, as sent.
pub fn request_body(prompt: &Prompt, wire_api: WireApi) -> Value {
    match wire_api {
        WireApi::Responses => {
            serde_json::to_value(ResponsesModelClient::build_request(prompt)).unwrap_or(Value::Null)
        }
        WireApi::ChatCompletions => build_chat_request(prompt),
    }
}

/// Wraps a client and appends each request body (in `wire_api`'s format) as a JSON line to a file.
pub struct RequestLog {
    inner: Arc<dyn ModelClient>,
    wire_api: WireApi,
    file: Mutex<std::fs::File>,
}

impl RequestLog {
    pub fn new(
        inner: Arc<dyn ModelClient>,
        wire_api: WireApi,
        path: &Path,
    ) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self {
            inner,
            wire_api,
            file: Mutex::new(file),
        })
    }
}

impl ModelClient for RequestLog {
    fn stream<'a>(&'a self, prompt: &'a Prompt) -> BoxFuture<'a, Result<ResponseStream, ApiError>> {
        if let Ok(mut line) = serde_json::to_vec(&request_body(prompt, self.wire_api)) {
            line.push(b'\n');
            if let Ok(mut file) = self.file.lock() {
                let _ = file.write_all(&line);
            }
        }
        self.inner.stream(prompt)
    }
}

/// [`ModelClient`] over the OpenAI Responses API (`POST /responses`).
pub struct ResponsesModelClient {
    client: ResponsesClient,
}

impl ResponsesModelClient {
    fn build_request(prompt: &Prompt) -> ResponsesApiRequest {
        let include = if prompt.reasoning.is_some() {
            vec!["reasoning.encrypted_content".to_string()]
        } else {
            Vec::new()
        };
        ResponsesApiRequest {
            model: prompt.model.clone(),
            stream: true,
            instructions: prompt.instructions.clone(),
            input: prompt.input.clone(),
            tools: prompt.tools.clone(),
            tool_choice: "auto".to_string(),
            parallel_tool_calls: prompt.parallel_tool_calls,
            reasoning: prompt.reasoning.clone(),
            store: false,
            include,
            text: create_text_param_for_request(
                prompt.verbosity,
                &prompt.output_schema,
                /*output_schema_strict*/ true,
            ),
        }
    }
}

impl ModelClient for ResponsesModelClient {
    fn stream<'a>(&'a self, prompt: &'a Prompt) -> BoxFuture<'a, Result<ResponseStream, ApiError>> {
        Box::pin(async move {
            let request = Self::build_request(prompt);
            self.client.stream_request(&request).await
        })
    }
}
