// New in ether: the model-client interface the engine calls, and its Responses API implementation.
//! A [`Prompt`] is everything one model request needs; a [`ModelClient`] turns it into a stream
//! of [`ResponseEvent`]s. The engine only sees this interface, so the wire format underneath
//! (Responses now, Chat Completions in Phase 4, or a replay file) can change without it knowing.

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
}

/// Streams model responses for prompts. Implementations hold no per-turn state.
pub trait ModelClient: Send + Sync {
    fn stream<'a>(&'a self, prompt: &'a Prompt) -> BoxFuture<'a, Result<ResponseStream, ApiError>>;
}

/// Errors building a model client from provider configuration.
#[derive(Debug, thiserror::Error)]
pub enum ClientBuildError {
    #[error("provider `{0}`: the Chat Completions wire API is not available yet")]
    ChatCompletionsUnavailable(String),
    #[error("provider `{name}`: {message}")]
    Provider { name: String, message: String },
}

/// [`ModelClient`] over the OpenAI Responses API (`POST /responses`).
pub struct ResponsesModelClient {
    client: ResponsesClient,
}

impl ResponsesModelClient {
    pub fn from_provider(info: &ModelProviderInfo) -> Result<Self, ClientBuildError> {
        if info.wire_api == WireApi::ChatCompletions {
            return Err(ClientBuildError::ChatCompletionsUnavailable(
                info.name.clone(),
            ));
        }
        let provider_error = |err: codex_protocol::error::CodexErr| ClientBuildError::Provider {
            name: info.name.clone(),
            message: err.to_string(),
        };
        let provider = info.to_api_provider(None).map_err(provider_error)?;
        let api_key = info.api_key().map_err(provider_error)?;
        let transport = ReqwestTransport::new(reqwest::Client::new());
        Ok(Self {
            client: ResponsesClient::new(transport, provider, api_key),
        })
    }

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
