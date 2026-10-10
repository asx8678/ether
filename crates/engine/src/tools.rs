// New in ether: tool routing; handlers are added as the tool pipeline is ported.
//! Turns model tool calls into tool outputs. Every call gets exactly one output.

use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
use serde_json::Value;

pub(crate) struct ToolRouter;

impl ToolRouter {
    pub(crate) fn new() -> Self {
        Self
    }

    /// Tool definitions in Responses API JSON form, in a stable order.
    pub(crate) fn specs(&self) -> Vec<Value> {
        Vec::new()
    }

    /// Runs one tool call and returns its output item.
    pub(crate) async fn dispatch(&self, call: &ResponseItem) -> ResponseItem {
        let (call_id, name) = match call {
            ResponseItem::FunctionCall { call_id, name, .. }
            | ResponseItem::CustomToolCall { call_id, name, .. } => (call_id.clone(), name.clone()),
            ResponseItem::LocalShellCall { call_id, .. } => (
                call_id.clone().unwrap_or_default(),
                "local_shell".to_string(),
            ),
            _ => (String::new(), String::new()),
        };
        output_for(call, call_id, format!("unsupported call: {name}"))
    }
}

/// Builds the output item that answers `call` with `text`.
pub(crate) fn output_for(call: &ResponseItem, call_id: String, text: String) -> ResponseItem {
    let output = FunctionCallOutputPayload::from_text(text);
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
