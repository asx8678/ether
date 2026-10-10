// New in ether: the `render_ui` tool (generative UI), shaped like the `update_plan` handler.
//! The model sends a view spec (see `ether_ui_spec`); the engine checks it and forwards it to
//! the client as `UiRender`. The tool result is only `ok` and the view id, so the history stays
//! small; an invalid spec comes back as the result, for the model to fix.

use crate::tools::ToolError;
use crate::tools::ToolHandler;
use crate::tools::ToolInvocation;
use crate::tools::ToolPayload;
use crate::tools::ToolResult;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::UiRenderEvent;
use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use futures::future::BoxFuture;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

/// One per session: numbers the views the model does not name.
#[derive(Default)]
pub(crate) struct RenderUiHandler {
    next_view: AtomicU64,
}

#[derive(Deserialize)]
struct RenderUiArgs {
    #[serde(default)]
    view_id: Option<String>,
    spec: Value,
}

impl ToolHandler for RenderUiHandler {
    fn spec(&self) -> ToolSpec {
        create_render_ui_tool()
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    fn handle(&self, invocation: ToolInvocation) -> BoxFuture<'_, ToolResult> {
        Box::pin(async move {
            let ToolPayload::Function { arguments } = &invocation.payload else {
                return Err(ToolError::RespondToModel(
                    "expected function-call arguments".to_string(),
                ));
            };
            let args: RenderUiArgs = serde_json::from_str(arguments).map_err(|err| {
                ToolError::RespondToModel(format!(
                    "invalid arguments: {err}. Send {{\"spec\": <node>}} and optionally \"view_id\"."
                ))
            })?;
            let node = ether_ui_spec::parse(&args.spec).map_err(|err| {
                ToolError::RespondToModel(format!(
                    "invalid spec, nothing was drawn. {err}. Fix the spec and call render_ui again."
                ))
            })?;
            let view_id = args
                .view_id
                .map(|id| id.trim().to_string())
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| {
                    format!(
                        "view-{}",
                        self.next_view.fetch_add(1, Ordering::Relaxed) + 1
                    )
                });
            let spec = serde_json::to_value(&node).map_err(|err| {
                ToolError::RespondToModel(format!("cannot encode the spec: {err}"))
            })?;
            invocation
                .sess
                .send_event(
                    &invocation.turn_id,
                    EventMsg::UiRender(UiRenderEvent {
                        call_id: invocation.call_id.clone(),
                        view_id: view_id.clone(),
                        spec,
                    }),
                )
                .await;
            let mut output = FunctionCallOutputPayload::from_text(format!("ok view_id={view_id}"));
            output.success = Some(true);
            Ok(output)
        })
    }
}

fn create_render_ui_tool() -> ToolSpec {
    let node_properties = BTreeMap::from([(
        "type".to_string(),
        JsonSchema::string_enum(
            vec![
                json!("column"),
                json!("row"),
                json!("text"),
                json!("markdown"),
                json!("table"),
                json!("gauge"),
            ],
            Some("The node kind; the other fields depend on it.".to_string()),
        ),
    )]);
    let mut spec = JsonSchema::object(
        node_properties,
        Some(vec!["type".to_string()]),
        Some(true.into()),
    );
    spec.description = Some("The root node of the view.".to_string());

    let properties = BTreeMap::from([
        (
            "view_id".to_string(),
            JsonSchema::string(Some(
                "Optional. Reuse the id of a view drawn earlier in this turn to replace it."
                    .to_string(),
            )),
        ),
        ("spec".to_string(), spec),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "render_ui".to_string(),
        description: format!(
            "Draws a small terminal view for the user: tables, gauges, text and markdown laid out \
             in rows and columns. Use it when structured data reads better as a view than as \
             prose. The result is only `ok view_id=...`; the user sees the view, you do not.\n\n{}",
            ether_ui_spec::SPEC_GUIDE
        ),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["spec".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}
