// Modified from OpenAI Codex (codex-rs/core/src/tools/handlers/request_user_input.rs and request_user_input_spec.rs @ 806d973): handler on ether's `ToolHandler`; available in every turn (ether has no collaboration modes, so no mode check and the request is always blocking); no sub-agent check or guardian retained-context recording.
//! `request_user_input`: ask the user one to three short multiple-choice questions and wait.

use crate::tools::ToolError;
use crate::tools::ToolHandler;
use crate::tools::ToolInvocation;
use crate::tools::ToolResult;
use crate::tools::parse_arguments;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::protocol::EventMsg;
use codex_protocol::request_user_input::RequestUserInputEvent;
use codex_protocol::request_user_input::RequestUserInputQuestion;
use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use futures::future::BoxFuture;
use serde::Deserialize;
use std::collections::BTreeMap;

const REQUEST_USER_INPUT_TOOL_NAME: &str = "request_user_input";

pub(crate) struct RequestUserInputHandler;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct RequestUserInputToolArgs {
    questions: Vec<RequestUserInputQuestion>,
}

impl ToolHandler for RequestUserInputHandler {
    fn spec(&self) -> ToolSpec {
        create_request_user_input_tool(
            "Request user input for one to three short questions and wait for the response."
                .to_string(),
        )
    }

    fn handle(&self, invocation: ToolInvocation) -> BoxFuture<'_, ToolResult> {
        Box::pin(async move {
            let args: RequestUserInputToolArgs = parse_arguments(&invocation.payload)?;
            let args =
                normalize_request_user_input_tool_args(args).map_err(ToolError::RespondToModel)?;
            let sess = &invocation.sess;
            // Register before asking, so an answer cannot arrive before anyone waits for it.
            let response = sess.request_user_input(invocation.call_id.clone()).await;
            sess.send_event(
                &invocation.turn_id,
                EventMsg::RequestUserInput(RequestUserInputEvent {
                    call_id: invocation.call_id.clone(),
                    turn_id: invocation.turn_id.clone(),
                    questions: args.questions,
                    is_blocking: true,
                    auto_resolution_ms: None,
                }),
            )
            .await;
            let response = response.await.map_err(|_| {
                ToolError::RespondToModel(format!(
                    "{REQUEST_USER_INPUT_TOOL_NAME} was cancelled before receiving a response"
                ))
            })?;
            let content = serde_json::to_string(&response).map_err(|err| {
                ToolError::RespondToModel(format!(
                    "failed to serialize {REQUEST_USER_INPUT_TOOL_NAME} response: {err}"
                ))
            })?;
            let mut output = FunctionCallOutputPayload::from_text(content);
            output.success = Some(true);
            Ok(output)
        })
    }
}

fn create_request_user_input_tool(description: String) -> ToolSpec {
    let option_props = BTreeMap::from([
        (
            "label".to_string(),
            JsonSchema::string(Some("User-facing label (1-5 words).".to_string())),
        ),
        (
            "description".to_string(),
            JsonSchema::string(Some(
                "One short sentence explaining impact/tradeoff if selected.".to_string(),
            )),
        ),
    ]);

    let options_schema = JsonSchema::array(JsonSchema::object(
            option_props,
            Some(vec!["label".to_string(), "description".to_string()]),
            Some(false.into()),
        ), Some(
            "Provide 2-3 mutually exclusive choices. Put the recommended option first and suffix its label with \"(Recommended)\". Do not include an \"Other\" option in this list; the client will add a free-form \"Other\" option automatically."
                .to_string(),
        ));

    let question_props = BTreeMap::from([
        (
            "id".to_string(),
            JsonSchema::string(Some(
                "Stable identifier for mapping answers (snake_case).".to_string(),
            )),
        ),
        (
            "header".to_string(),
            JsonSchema::string(Some(
                "Short header label shown in the UI (12 or fewer chars).".to_string(),
            )),
        ),
        (
            "question".to_string(),
            JsonSchema::string(Some(
                "Single-sentence prompt shown to the user.".to_string(),
            )),
        ),
        ("options".to_string(), options_schema),
    ]);

    let questions_schema = JsonSchema::array(
        JsonSchema::object(
            question_props,
            Some(vec![
                "id".to_string(),
                "header".to_string(),
                "question".to_string(),
                "options".to_string(),
            ]),
            Some(false.into()),
        ),
        Some("Questions to show the user. Prefer 1 and do not exceed 3".to_string()),
    );

    let properties = BTreeMap::from([("questions".to_string(), questions_schema)]);

    ToolSpec::Function(ResponsesApiTool {
        name: REQUEST_USER_INPUT_TOOL_NAME.to_string(),
        description,
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["questions".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}

fn normalize_request_user_input_tool_args(
    mut args: RequestUserInputToolArgs,
) -> Result<RequestUserInputToolArgs, String> {
    let missing_options = args
        .questions
        .iter()
        .any(|question| question.options.as_ref().is_none_or(Vec::is_empty));
    if missing_options {
        return Err("request_user_input requires non-empty options for every question".to_string());
    }

    for question in &mut args.questions {
        question.is_other = true;
    }

    Ok(args)
}
