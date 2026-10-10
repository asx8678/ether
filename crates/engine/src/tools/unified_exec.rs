// Modified from OpenAI Codex (codex-rs/core/src/tools/handlers/unified_exec.rs and unified_exec/{exec_command,write_stdin}.rs @ 806d973): handlers on ether's `ToolHandler`; local environment only; no login/environment_id/additional_permissions/prefix_rule arguments, apply_patch interception, hooks, skills or one-shot mode.
//! `exec_command` and `write_stdin`.

use crate::exec::CallContext;
use crate::exec::ExecCommandParams;
use crate::exec::WriteStdinParams;
use crate::tools::ToolError;
use crate::tools::ToolHandler;
use crate::tools::ToolInvocation;
use crate::tools::ToolResult;
use crate::tools::parse_arguments;
use crate::tools::shell_spec::create_exec_command_tool;
use crate::tools::shell_spec::create_write_stdin_tool;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::SandboxPermissions;
use codex_tools::ToolSpec;
use futures::future::BoxFuture;
use serde::Deserialize;

/// Model-facing rejections are kept short.
const REJECTION_MAX_CHARS: usize = 900;

#[derive(Debug, Deserialize)]
struct ExecCommandArgs {
    cmd: String,
    #[serde(default)]
    workdir: Option<String>,
    #[serde(default)]
    shell: Option<String>,
    #[serde(default)]
    tty: bool,
    #[serde(default = "default_exec_yield_time_ms")]
    yield_time_ms: u64,
    #[serde(default)]
    max_output_tokens: Option<usize>,
    #[serde(default)]
    sandbox_permissions: Option<SandboxPermissions>,
    #[serde(default)]
    justification: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WriteStdinArgs {
    // The model is trained on `session_id`.
    session_id: i32,
    #[serde(default)]
    chars: String,
    #[serde(default = "default_write_stdin_yield_time_ms")]
    yield_time_ms: u64,
    #[serde(default)]
    max_output_tokens: Option<usize>,
}

fn default_exec_yield_time_ms() -> u64 {
    10_000
}

fn default_write_stdin_yield_time_ms() -> u64 {
    250
}

fn rejection(prefix: &str, message: String) -> ToolError {
    let message = format!("{prefix}{message}");
    ToolError::RespondToModel(codex_utils_string::truncate_middle_chars(
        &message,
        REJECTION_MAX_CHARS,
    ))
}

pub(crate) struct ExecCommandHandler;

impl ToolHandler for ExecCommandHandler {
    fn spec(&self) -> ToolSpec {
        create_exec_command_tool()
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    fn handle(&self, invocation: ToolInvocation) -> BoxFuture<'_, ToolResult> {
        Box::pin(async move {
            let args: ExecCommandArgs = parse_arguments(&invocation.payload)?;
            let sandbox_permissions = args.sandbox_permissions.unwrap_or_default();
            if sandbox_permissions.uses_additional_permissions() {
                return Err(ToolError::RespondToModel(
                    "`with_additional_permissions` is not supported; use `require_escalated`."
                        .to_string(),
                ));
            }
            let ctx = CallContext {
                sess: &invocation.sess,
                turn_id: &invocation.turn_id,
                call_id: &invocation.call_id,
                cancel: &invocation.cancel,
            };
            let output = invocation
                .sess
                .exec
                .exec_command(
                    &ctx,
                    ExecCommandParams {
                        cmd: args.cmd,
                        shell: args.shell,
                        workdir: args.workdir,
                        tty: args.tty,
                        yield_time_ms: args.yield_time_ms,
                        max_output_tokens: args.max_output_tokens,
                        sandbox_permissions,
                        justification: args.justification,
                    },
                )
                .await
                .map_err(|err| rejection("", err))?;
            let text = output.response_text(invocation.sess.config.model.truncation_policy);
            let mut payload = FunctionCallOutputPayload::from_text(text);
            payload.success = Some(true);
            Ok(payload)
        })
    }
}

pub(crate) struct WriteStdinHandler;

impl ToolHandler for WriteStdinHandler {
    fn spec(&self) -> ToolSpec {
        create_write_stdin_tool()
    }

    fn supports_parallel(&self) -> bool {
        true
    }

    fn handle(&self, invocation: ToolInvocation) -> BoxFuture<'_, ToolResult> {
        Box::pin(async move {
            let args: WriteStdinArgs = parse_arguments(&invocation.payload)?;
            let ctx = CallContext {
                sess: &invocation.sess,
                turn_id: &invocation.turn_id,
                call_id: &invocation.call_id,
                cancel: &invocation.cancel,
            };
            let output = invocation
                .sess
                .exec
                .write_stdin(
                    &ctx,
                    WriteStdinParams {
                        session_id: args.session_id,
                        chars: args.chars,
                        yield_time_ms: args.yield_time_ms,
                        max_output_tokens: args.max_output_tokens,
                    },
                )
                .await
                .map_err(|err| rejection(&format!("{} failed: ", invocation.tool_name), err))?;
            let text = output.response_text(invocation.sess.config.model.truncation_policy);
            let mut payload = FunctionCallOutputPayload::from_text(text);
            payload.success = Some(true);
            Ok(payload)
        })
    }
}
