// New in ether: tool routing, in the role of Codex's tool registry, router and parallel runtime (codex-rs/core/src/tools/{registry,router,parallel}.rs @ 806d973).
//! Turns model tool calls into tool outputs. Every call gets exactly one output.
//!
//! A tool is a [`ToolHandler`]: it advertises a [`ToolSpec`] and handles invocations. Handlers
//! that say they are parallel-safe run concurrently; the others run one at a time, after any
//! parallel calls already in flight (a read/write lock, as in Codex's `parallel.rs`).

mod apply_patch;
mod plan;
mod render_ui;
mod request_user_input;
mod shell_spec;
mod unified_exec;
mod view_image;

use crate::config::EngineConfig;
use crate::session::Session;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
use codex_tools::ToolSpec;
use codex_tools::create_tools_json_for_responses_api;
use futures::future::BoxFuture;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use tracing::error;

/// What the model sent for a call.
#[derive(Debug, Clone)]
pub(crate) enum ToolPayload {
    /// A function tool call with JSON arguments.
    Function { arguments: String },
    /// A freeform (custom) tool call with raw input.
    Custom { input: String },
}

/// One tool call, with everything a handler needs to run it.
pub(crate) struct ToolInvocation {
    pub(crate) sess: Arc<Session>,
    pub(crate) turn_id: String,
    pub(crate) call_id: String,
    pub(crate) tool_name: String,
    pub(crate) payload: ToolPayload,
    /// Cancelled when the turn is interrupted.
    pub(crate) cancel: CancellationToken,
}

/// A failure the model should see as the tool's output.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ToolError {
    #[error("{0}")]
    RespondToModel(String),
}

pub(crate) type ToolResult = Result<FunctionCallOutputPayload, ToolError>;

pub(crate) trait ToolHandler: Send + Sync {
    /// The definition advertised to the model. Must be stable across calls.
    fn spec(&self) -> ToolSpec;

    /// Whether calls may run concurrently with other parallel-safe calls.
    fn supports_parallel(&self) -> bool {
        false
    }

    fn handle(&self, invocation: ToolInvocation) -> BoxFuture<'_, ToolResult>;
}

/// The built-in tools, in the order they are advertised. Each tool module adds its handler here.
pub(crate) fn built_in_handlers(config: &EngineConfig) -> Vec<Arc<dyn ToolHandler>> {
    let mut handlers: Vec<Arc<dyn ToolHandler>> = vec![
        Arc::new(unified_exec::ExecCommandHandler),
        Arc::new(unified_exec::WriteStdinHandler),
        Arc::new(plan::PlanHandler),
        Arc::new(apply_patch::ApplyPatchHandler::new(
            apply_patch::ApplyPatchToolType::for_model(&config.model.slug),
        )),
        Arc::new(view_image::ViewImageHandler),
    ];
    if config.user_input_tool {
        handlers.push(Arc::new(request_user_input::RequestUserInputHandler));
    }
    if config.offers_render_ui() {
        handlers.push(Arc::new(render_ui::RenderUiHandler::default()));
    }
    handlers
}

pub(crate) struct ToolRouter {
    /// Handlers in registration order; the advertised tool list follows this order.
    handlers: Vec<Arc<dyn ToolHandler>>,
    /// Parallel-safe calls hold it shared, the others exclusively.
    gate: Arc<RwLock<()>>,
}

impl ToolRouter {
    pub(crate) fn new(handlers: Vec<Arc<dyn ToolHandler>>) -> Self {
        Self {
            handlers,
            gate: Arc::new(RwLock::new(())),
        }
    }

    /// Tool definitions in Responses API JSON form, in a stable order.
    pub(crate) fn specs(&self) -> Vec<Value> {
        let specs: Vec<ToolSpec> = self.handlers.iter().map(|handler| handler.spec()).collect();
        create_tools_json_for_responses_api(&specs).unwrap_or_else(|err| {
            error!("failed to serialize tool specs: {err}");
            Vec::new()
        })
    }

    fn handler(&self, name: &str) -> Option<&Arc<dyn ToolHandler>> {
        self.handlers
            .iter()
            .find(|handler| handler.spec().name() == name)
    }

    /// Runs one tool call and returns its output item.
    pub(crate) async fn dispatch(
        &self,
        sess: Arc<Session>,
        turn_id: String,
        call: &ResponseItem,
        cancel: CancellationToken,
    ) -> ResponseItem {
        let Some((call_id, tool_name, payload)) = parse_call(call) else {
            return output_for(call, String::new(), "unsupported call".to_string());
        };
        let Some(handler) = self.handler(&tool_name) else {
            return output_for(call, call_id, format!("unsupported call: {tool_name}"));
        };
        let invocation = ToolInvocation {
            sess,
            turn_id,
            call_id: call_id.clone(),
            tool_name,
            payload,
            cancel,
        };
        let result = if handler.supports_parallel() {
            let _shared = Arc::clone(&self.gate).read_owned().await;
            handler.handle(invocation).await
        } else {
            let _exclusive = Arc::clone(&self.gate).write_owned().await;
            handler.handle(invocation).await
        };
        let output = match result {
            Ok(output) => output,
            Err(ToolError::RespondToModel(message)) => {
                let mut output = FunctionCallOutputPayload::from_text(message);
                output.success = Some(false);
                output
            }
        };
        output_item(call, call_id, output)
    }
}

fn parse_call(call: &ResponseItem) -> Option<(String, String, ToolPayload)> {
    match call {
        ResponseItem::FunctionCall {
            call_id,
            name,
            arguments,
            ..
        } => Some((
            call_id.clone(),
            name.clone(),
            ToolPayload::Function {
                arguments: arguments.clone(),
            },
        )),
        ResponseItem::CustomToolCall {
            call_id,
            name,
            input,
            ..
        } => Some((
            call_id.clone(),
            name.clone(),
            ToolPayload::Custom {
                input: input.clone(),
            },
        )),
        _ => None,
    }
}

/// The call id of a tool call item.
pub(crate) fn call_id_of(call: &ResponseItem) -> String {
    match call {
        ResponseItem::FunctionCall { call_id, .. }
        | ResponseItem::CustomToolCall { call_id, .. } => call_id.clone(),
        ResponseItem::LocalShellCall { call_id, .. } => call_id.clone().unwrap_or_default(),
        _ => String::new(),
    }
}

/// Builds the output item that answers `call` with `text`.
pub(crate) fn output_for(call: &ResponseItem, call_id: String, text: String) -> ResponseItem {
    output_item(call, call_id, FunctionCallOutputPayload::from_text(text))
}

fn output_item(
    call: &ResponseItem,
    call_id: String,
    output: FunctionCallOutputPayload,
) -> ResponseItem {
    match call {
        ResponseItem::CustomToolCall { .. } => ResponseItem::CustomToolCallOutput {
            id: None,
            call_id,
            name: None,
            output,
            internal_chat_message_metadata_passthrough: None,
        },
        _ => ResponseItem::FunctionCallOutput {
            id: None,
            call_id: Some(call_id),
            name: None,
            namespace: None,
            output,
            internal_chat_message_metadata_passthrough: None,
        },
    }
}

/// Parses function-call JSON arguments into `T`, as a model-visible error on failure.
pub(crate) fn parse_arguments<T: serde::de::DeserializeOwned>(
    payload: &ToolPayload,
) -> Result<T, ToolError> {
    let ToolPayload::Function { arguments } = payload else {
        return Err(ToolError::RespondToModel(
            "expected function-call arguments".to_string(),
        ));
    };
    serde_json::from_str(arguments).map_err(|err| {
        ToolError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })
}
