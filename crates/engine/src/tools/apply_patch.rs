// Modified from OpenAI Codex (codex-rs/core/src/tools/handlers/apply_patch.rs, apply_patch_spec.rs, core/src/apply_patch.rs and core/src/tools/runtimes/apply_patch.rs @ 806d973): handler on ether's `ToolHandler`; adds a JSON function variant (`{"input": ...}`) beside the freeform one; the approval flow and patch application are inline instead of going through the tool orchestrator; patches apply in-process on `LOCAL_FS` with no sandboxed helper (decision C2); no hooks, streaming argument diffs, environment selection, granted-permission widening or telemetry.
//! `apply_patch`: edit files with a patch in Codex's patch format.

use crate::safety::PatchPermissions;
use crate::safety::SafetyCheck;
use codex_model_provider_info::WireApi;
use crate::safety::assess_patch_safety;
use crate::session::Session;
use crate::tools::ToolError;
use crate::tools::ToolHandler;
use crate::tools::ToolInvocation;
use crate::tools::ToolPayload;
use crate::tools::ToolResult;
use crate::tools::parse_arguments;
use codex_apply_patch::ApplyPatchAction;
use codex_apply_patch::ApplyPatchFileChange;
use codex_apply_patch::ApplyPatchOptions;
use codex_apply_patch::MaybeApplyPatchVerified;
use codex_file_system::LOCAL_FS;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::protocol::ApplyPatchApprovalRequestEvent;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::FileChange;
use codex_protocol::protocol::PatchApplyBeginEvent;
use codex_protocol::protocol::PatchApplyEndEvent;
use codex_protocol::protocol::PatchApplyStatus;
use codex_protocol::protocol::ReviewDecision;
use codex_tools::FreeformTool;
use codex_tools::FreeformToolFormat;
use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use codex_utils_path_uri::PathUri;
use futures::future::BoxFuture;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

const APPLY_PATCH_TOOL_NAME: &str = "apply_patch";
const APPLY_PATCH_LARK_GRAMMAR: &str = include_str!("../../assets/apply_patch.lark");

const APPLY_PATCH_FUNCTION_DESCRIPTION: &str = r#"Use the `apply_patch` tool to edit files. Pass the whole patch as the `input` string.
The patch format is a stripped-down, file-oriented diff:

*** Begin Patch
[ one or more file sections ]
*** End Patch

Each file section starts with one of three headers:
*** Add File: <path> - create a new file. Every following line is a + line (the initial contents).
*** Delete File: <path> - remove an existing file. Nothing follows.
*** Update File: <path> - patch an existing file in place (optionally followed by `*** Move to: <new path>` to rename it).

An update is one or more hunks, each introduced by `@@` (optionally followed by a class or function name to locate the hunk). Within a hunk each line starts with:
" " for context, "-" for a removed line, "+" for an added line.
Show 3 lines of context above and below each change, and do not repeat context lines between adjacent changes. Use `@@ <name>` when 3 lines are not enough to find the place uniquely.

Example:
*** Begin Patch
*** Add File: hello.txt
+Hello world
*** Update File: src/app.py
*** Move to: src/main.py
@@ def greet():
-print("Hi")
+print("Hello, world!")
*** Delete File: obsolete.txt
*** End Patch

File paths must be relative, never absolute."#;

/// How `apply_patch` is advertised to the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApplyPatchToolType {
    /// A function tool whose patch is the `input` string argument (works with any provider).
    Function,
    /// A freeform (custom) tool constrained by a Lark grammar (OpenAI models).
    Freeform,
}

pub(crate) struct ApplyPatchHandler {
    tool_type: ApplyPatchToolType,
}

impl ApplyPatchToolType {
    /// Freeform for GPT-5 models, which Codex serves the freeform tool; a JSON function for
    /// everything else (GLM and other OpenAI-compatible providers).
    pub(crate) fn for_model(slug: &str, wire_api: WireApi) -> Self {
        // Chat Completions has function tools only.
        if slug.starts_with("gpt-5") && wire_api == WireApi::Responses {
            Self::Freeform
        } else {
            Self::Function
        }
    }
}

impl ApplyPatchHandler {
    pub(crate) fn new(tool_type: ApplyPatchToolType) -> Self {
        Self { tool_type }
    }
}

#[derive(Deserialize)]
struct ApplyPatchFunctionArgs {
    input: String,
}

impl ToolHandler for ApplyPatchHandler {
    fn spec(&self) -> ToolSpec {
        match self.tool_type {
            ApplyPatchToolType::Function => create_apply_patch_function_tool(),
            ApplyPatchToolType::Freeform => create_apply_patch_freeform_tool(),
        }
    }

    fn handle(&self, invocation: ToolInvocation) -> BoxFuture<'_, ToolResult> {
        Box::pin(async move {
            // Both call shapes are accepted whichever one is advertised.
            let patch_input = match &invocation.payload {
                ToolPayload::Custom { input } => input.clone(),
                ToolPayload::Function { .. } => {
                    parse_arguments::<ApplyPatchFunctionArgs>(&invocation.payload)?.input
                }
            };
            let args = codex_apply_patch::parse_patch(&patch_input).map_err(|parse_error| {
                ToolError::RespondToModel(format!("apply_patch verification failed: {parse_error}"))
            })?;
            let cwd = PathUri::from_abs_path(&invocation.sess.config.cwd);
            match codex_apply_patch::verify_apply_patch_args(args, &cwd, LOCAL_FS.as_ref(), None)
                .await
            {
                MaybeApplyPatchVerified::Body(action) => {
                    execute_verified_patch(
                        &invocation.sess,
                        &invocation.turn_id,
                        &invocation.call_id,
                        action,
                    )
                    .await
                }
                MaybeApplyPatchVerified::CorrectnessError(parse_error) => {
                    Err(ToolError::RespondToModel(format!(
                        "apply_patch verification failed: {parse_error}"
                    )))
                }
                MaybeApplyPatchVerified::ShellParseError(error) => {
                    tracing::trace!("Failed to parse apply_patch input, {error:?}");
                    Err(ToolError::RespondToModel(
                        "apply_patch handler received invalid patch input".to_string(),
                    ))
                }
                MaybeApplyPatchVerified::NotApplyPatch => Err(ToolError::RespondToModel(
                    "apply_patch handler received non-apply_patch input".to_string(),
                )),
            }
        })
    }
}

/// Runs `apply_patch` invoked through a shell command (`apply_patch <<'EOF' ... EOF`), as
/// Codex's `intercept_apply_patch` does for its shell tools. Returns `Ok(None)` when `command`
/// is not an `apply_patch` invocation, so the caller runs it as a normal command.
#[allow(dead_code)] // For the exec_command handler, which is ported separately.
pub(crate) async fn intercept_apply_patch(
    command: &[String],
    sess: &Arc<Session>,
    turn_id: &str,
    call_id: &str,
) -> Result<Option<FunctionCallOutputPayload>, ToolError> {
    let cwd = PathUri::from_abs_path(&sess.config.cwd);
    match codex_apply_patch::maybe_parse_apply_patch_verified(
        command,
        &cwd,
        LOCAL_FS.as_ref(),
        None,
    )
    .await
    {
        MaybeApplyPatchVerified::Body(action) => {
            execute_verified_patch(sess, turn_id, call_id, action)
                .await
                .map(Some)
        }
        MaybeApplyPatchVerified::CorrectnessError(parse_error) => Err(ToolError::RespondToModel(
            format!("apply_patch verification failed: {parse_error}"),
        )),
        MaybeApplyPatchVerified::ShellParseError(error) => {
            tracing::trace!("Failed to parse apply_patch input, {error:?}");
            Ok(None)
        }
        MaybeApplyPatchVerified::NotApplyPatch => Ok(None),
    }
}

/// Checks a verified patch against the sandbox mode and approval policy, asks the user when
/// needed, applies it and reports `PatchApplyBegin`/`PatchApplyEnd`.
async fn execute_verified_patch(
    sess: &Arc<Session>,
    turn_id: &str,
    call_id: &str,
    action: ApplyPatchAction,
) -> ToolResult {
    let config = &sess.config;
    let permissions = PatchPermissions::new(config.sandbox_mode, &config.cwd);
    let auto_approved = match assess_patch_safety(&action, config.approval_policy, &permissions)
        .map_err(|error| {
            ToolError::RespondToModel(format!("failed to check patch permissions: {error}"))
        })? {
        SafetyCheck::AutoApprove => true,
        // Files the user already approved for the session need no new question.
        SafetyCheck::AskUser => {
            let approved = sess.patch_approved_paths.lock().await;
            patch_paths(&action).iter().all(|path| approved.contains(path))
        }
        SafetyCheck::Reject { reason } => {
            return Err(ToolError::RespondToModel(format!(
                "patch rejected: {reason}"
            )));
        }
    };
    let changes = convert_apply_patch_to_protocol(&action);
    if !auto_approved && !request_patch_approval(sess, turn_id, call_id, &action, &changes).await {
        emit_patch_end(
            sess,
            turn_id,
            call_id,
            changes,
            String::new(),
            String::new(),
            PatchApplyStatus::Declined,
        )
        .await;
        return Err(ToolError::RespondToModel(
            "patch rejected by user".to_string(),
        ));
    }

    // Begin follows the approval decision, as in Codex.
    sess.send_event(
        turn_id,
        EventMsg::PatchApplyBegin(PatchApplyBeginEvent {
            call_id: call_id.to_string(),
            turn_id: turn_id.to_string(),
            auto_approved,
            changes: changes.clone(),
        }),
    )
    .await;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = codex_apply_patch::apply_patch_with_options(
        &action.patch,
        ApplyPatchOptions {
            follow_symlinks: true,
        },
        &action.cwd,
        &mut stdout,
        &mut stderr,
        LOCAL_FS.as_ref(),
        None,
    )
    .await;
    let stdout = String::from_utf8_lossy(&stdout).into_owned();
    let stderr = String::from_utf8_lossy(&stderr).into_owned();
    let (success, delta) = match result {
        Ok(delta) => (true, delta),
        Err(failure) => (false, failure.into_parts().1),
    };
    // Changes committed before a failure still count toward the turn diff.
    sess.turn_diffs.lock().await.record(
        turn_id,
        &PathUri::from_abs_path(&config.cwd),
        Some(&delta),
    );

    let status = if success {
        PatchApplyStatus::Completed
    } else {
        PatchApplyStatus::Failed
    };
    let content = format!("{stdout}{stderr}");
    emit_patch_end(sess, turn_id, call_id, changes, stdout, stderr, status).await;
    let mut output = FunctionCallOutputPayload::from_text(content);
    output.success = Some(success);
    Ok(output)
}

/// The files a patch writes: changed paths and move destinations.
fn patch_paths(action: &ApplyPatchAction) -> Vec<PathBuf> {
    convert_apply_patch_to_protocol(action)
        .into_iter()
        .flat_map(|(path, change)| {
            let moved = match change {
                FileChange::Update {
                    move_path: Some(to),
                    ..
                } => Some(to),
                _ => None,
            };
            std::iter::once(path).chain(moved)
        })
        .collect()
}

/// Asks the client to approve the patch. Returns whether it may be applied; a dropped request
/// (the turn ended) counts as a rejection. "Yes for this session" remembers the patch's files.
async fn request_patch_approval(
    sess: &Session,
    turn_id: &str,
    call_id: &str,
    action: &ApplyPatchAction,
    changes: &HashMap<PathBuf, FileChange>,
) -> bool {
    let decision = sess.request_approval(call_id.to_string()).await;
    sess.send_event(
        turn_id,
        EventMsg::ApplyPatchApprovalRequest(ApplyPatchApprovalRequestEvent {
            call_id: call_id.to_string(),
            turn_id: turn_id.to_string(),
            started_at_ms: chrono::Utc::now().timestamp_millis(),
            changes: changes.clone(),
            reason: None,
            grant_root: None,
        }),
    )
    .await;
    match decision.await {
        Ok(ReviewDecision::ApprovedForSession) => {
            sess.patch_approved_paths
                .lock()
                .await
                .extend(patch_paths(action));
            true
        }
        Ok(ReviewDecision::Approved | ReviewDecision::ApprovedExecpolicyAmendment { .. }) => true,
        Ok(_) | Err(_) => false,
    }
}

async fn emit_patch_end(
    sess: &Session,
    turn_id: &str,
    call_id: &str,
    changes: HashMap<PathBuf, FileChange>,
    stdout: String,
    stderr: String,
    status: PatchApplyStatus,
) {
    sess.send_event(
        turn_id,
        EventMsg::PatchApplyEnd(PatchApplyEndEvent {
            call_id: call_id.to_string(),
            turn_id: turn_id.to_string(),
            stdout,
            stderr,
            success: status == PatchApplyStatus::Completed,
            changes,
            status,
        }),
    )
    .await;
}

fn convert_apply_patch_to_protocol(action: &ApplyPatchAction) -> HashMap<PathBuf, FileChange> {
    let mut result = HashMap::with_capacity(action.changes().len());
    for (path, change) in action.changes() {
        let protocol_change = match change {
            ApplyPatchFileChange::Add { content, .. } => FileChange::Add {
                content: content.clone(),
            },
            ApplyPatchFileChange::Delete { content } => FileChange::Delete {
                content: content.clone(),
            },
            ApplyPatchFileChange::Update {
                unified_diff,
                move_path,
                new_content: _new_content,
            } => FileChange::Update {
                unified_diff: unified_diff.clone(),
                move_path: move_path.as_ref().map(PathUri::to_path_buf),
            },
        };
        result.insert(path.to_path_buf(), protocol_change);
    }
    result
}

/// Returns a custom tool that can be used to edit files. Well-suited for GPT-5 models
/// https://platform.openai.com/docs/guides/function-calling#custom-tools
fn create_apply_patch_freeform_tool() -> ToolSpec {
    ToolSpec::Freeform(FreeformTool {
        name: APPLY_PATCH_TOOL_NAME.to_string(),
        description: "The `apply_patch` tool can be used to edit files. This is a FREEFORM tool, so do not wrap the patch in JSON.".to_string(),
        defer_loading: None,
        format: FreeformToolFormat {
            r#type: "grammar".to_string(),
            syntax: "lark".to_string(),
            definition: APPLY_PATCH_LARK_GRAMMAR.to_string(),
        },
    })
}

/// The same tool as a JSON function, for providers without custom tools.
fn create_apply_patch_function_tool() -> ToolSpec {
    let properties = BTreeMap::from([(
        "input".to_string(),
        JsonSchema::string(Some(
            "The entire contents of the apply_patch command".to_string(),
        )),
    )]);
    ToolSpec::Function(ResponsesApiTool {
        name: APPLY_PATCH_TOOL_NAME.to_string(),
        description: APPLY_PATCH_FUNCTION_DESCRIPTION.to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["input".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}
