// Modified from OpenAI Codex (codex-rs/core/src/tools/handlers/plan.rs and plan_spec.rs @ 806d973): handler on ether's `ToolHandler`; Plan-mode check removed (ether has no Plan mode).
//! `update_plan`: the model's checklist, forwarded to the client as `PlanUpdate`.

use crate::tools::ToolHandler;
use crate::tools::ToolInvocation;
use crate::tools::ToolResult;
use crate::tools::parse_arguments;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_protocol::protocol::EventMsg;
use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use futures::future::BoxFuture;
use serde_json::json;
use std::collections::BTreeMap;

const PLAN_UPDATED_MESSAGE: &str = "Plan updated";

pub(crate) struct PlanHandler;

impl ToolHandler for PlanHandler {
    fn spec(&self) -> ToolSpec {
        create_update_plan_tool()
    }

    fn handle(&self, invocation: ToolInvocation) -> BoxFuture<'_, ToolResult> {
        Box::pin(async move {
            let args: UpdatePlanArgs = parse_arguments(&invocation.payload)?;
            invocation
                .sess
                .send_event(&invocation.turn_id, EventMsg::PlanUpdate(args))
                .await;
            let mut output = FunctionCallOutputPayload::from_text(PLAN_UPDATED_MESSAGE.to_string());
            output.success = Some(true);
            Ok(output)
        })
    }
}

fn create_update_plan_tool() -> ToolSpec {
    let plan_item_properties = BTreeMap::from([
        (
            "step".to_string(),
            JsonSchema::string(Some("Task step text.".to_string())),
        ),
        (
            "status".to_string(),
            JsonSchema::string_enum(
                vec![json!("pending"), json!("in_progress"), json!("completed")],
                Some("Step status.".to_string()),
            ),
        ),
    ]);

    let properties = BTreeMap::from([
        (
            "explanation".to_string(),
            JsonSchema::string(Some(
                "Optional explanation for this plan update.".to_string(),
            )),
        ),
        (
            "plan".to_string(),
            JsonSchema::array(
                JsonSchema::object(
                    plan_item_properties,
                    Some(vec!["step".to_string(), "status".to_string()]),
                    Some(false.into()),
                ),
                Some("The list of steps".to_string()),
            ),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "update_plan".to_string(),
        description: r#"Updates the task plan.
Provide an optional explanation and a list of plan items, each with a step and status.
At most one step can be in_progress at a time.
"#
        .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["plan".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}
