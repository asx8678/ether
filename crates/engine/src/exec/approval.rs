// Modified from OpenAI Codex (codex-rs/core/src/tools/sandboxing.rs, tools/approvals.rs and command_canonicalization.rs @ 806d973): the approval primitives used by exec only; no guardian, hooks, telemetry, network approval or execpolicy amendments; PowerShell canonicalization removed; approvals go through `Session::request_approval`.
//! Approval requirements, the per-session approval cache, and asking the client.

use crate::session::Session;
use codex_protocol::approvals::ExecApprovalKind;
use codex_protocol::approvals::ExecApprovalRequestEvent;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ReviewDecision;
use codex_protocol::sandbox::SandboxOverride;
use codex_shell_command::bash::extract_bash_command;
use codex_shell_command::bash::parse_shell_lc_plain_commands;
use codex_shell_command::parse_command::parse_command;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Serialize;
use std::collections::HashSet;
use tokio_util::sync::CancellationToken;

const CANONICAL_BASH_SCRIPT_PREFIX: &str = "__codex_shell_script__";

/// What the orchestrator should do with a command before running it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExecApprovalRequirement {
    /// No approval required.
    Skip {
        /// The first attempt should skip sandboxing (explicitly allowed by a rule).
        bypass_sandbox: bool,
    },
    /// Approval required.
    NeedsApproval { reason: Option<String> },
    /// Execution forbidden.
    Forbidden { reason: String },
}

pub(crate) fn sandbox_override_for_first_attempt(
    sandbox_permissions: SandboxPermissions,
    exec_approval_requirement: &ExecApprovalRequirement,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    already_approved: bool,
) -> SandboxOverride {
    // Only actual approval of an explicit escalation may widen the filesystem;
    // a command allow rule does not authorize removing filesystem restrictions.
    if !unsandboxed_execution_allowed(file_system_sandbox_policy) {
        return if sandbox_permissions.requires_escalated_permissions() && already_approved {
            SandboxOverride::EscalatedSandboxWithRestrictions
        } else {
            SandboxOverride::NoOverride
        };
    }

    // ExecPolicy `Allow` can intentionally imply full trust (Skip + bypass_sandbox=true).
    if matches!(
        exec_approval_requirement,
        ExecApprovalRequirement::Skip {
            bypass_sandbox: true
        }
    ) {
        return SandboxOverride::BypassSandboxFirstAttempt;
    }

    if sandbox_permissions.requires_escalated_permissions() {
        SandboxOverride::BypassSandboxFirstAttempt
    } else {
        SandboxOverride::NoOverride
    }
}

/// Returns true when the active filesystem policy can be represented by running without a
/// filesystem sandbox. Denied reads only exist inside the sandbox, so a policy with any
/// denied-read paths must stay sandboxed.
pub(crate) fn unsandboxed_execution_allowed(
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
) -> bool {
    !file_system_sandbox_policy.has_denied_read_restrictions()
}

/// Whether a sandbox denial may be retried without the sandbox after asking.
pub(crate) fn wants_no_sandbox_approval(policy: AskForApproval) -> bool {
    match policy {
        AskForApproval::UnlessTrusted => true,
        AskForApproval::Never | AskForApproval::OnRequest => false,
        AskForApproval::Granular(granular_config) => granular_config.sandbox_approval,
    }
}

/// Whether the unsandboxed retry may skip asking.
pub(crate) fn should_bypass_approval(policy: AskForApproval, already_approved: bool) -> bool {
    already_approved || matches!(policy, AskForApproval::Never)
}

/// Canonicalize command argv for approval-cache matching.
///
/// This keeps approval decisions stable across wrapper-path differences (for
/// example `/bin/bash -lc` vs `bash -lc`) while preserving exact script text for
/// complex scripts where we cannot safely recover a tokenized command sequence.
pub(crate) fn canonicalize_command_for_approval(command: &[String]) -> Vec<String> {
    if let Some(commands) = parse_shell_lc_plain_commands(command)
        && let [single_command] = commands.as_slice()
    {
        return single_command.clone();
    }

    if let Some((_shell, script)) = extract_bash_command(command) {
        let shell_mode = command.get(1).cloned().unwrap_or_default();
        return vec![
            CANONICAL_BASH_SCRIPT_PREFIX.to_string(),
            shell_mode,
            script.to_string(),
        ];
    }

    command.to_vec()
}

/// What an approval is remembered by, as in Codex's `UnifiedExecApprovalKey`.
#[derive(Serialize)]
pub(crate) struct ApprovalKey<'a> {
    pub(crate) command: Vec<String>,
    pub(crate) cwd: &'a AbsolutePathBuf,
    pub(crate) tty: bool,
    pub(crate) sandbox_permissions: SandboxPermissions,
}

/// Commands approved for the rest of the session.
#[derive(Default)]
pub(crate) struct ApprovalStore {
    approved_for_session: HashSet<String>,
}

impl ApprovalStore {
    pub(crate) fn is_approved(&self, key: &ApprovalKey<'_>) -> bool {
        serde_json::to_string(key)
            .ok()
            .is_some_and(|key| self.approved_for_session.contains(&key))
    }

    pub(crate) fn approve(&mut self, key: &ApprovalKey<'_>) {
        if let Ok(key) = serde_json::to_string(key) {
            self.approved_for_session.insert(key);
        }
    }
}

/// The client's answer, reduced to what exec needs.
pub(crate) enum ApprovalOutcome {
    Approved,
    ApprovedForSession,
    /// Not approved; the message is shown to the model.
    Rejected(String),
}

/// Asks the client to approve `command` and waits for the answer. Interrupting the turn (or the
/// client dropping the request) counts as an abort.
pub(crate) async fn request_command_approval(
    sess: &Session,
    turn_id: &str,
    call_id: &str,
    command: &[String],
    cwd: &AbsolutePathBuf,
    reason: Option<String>,
    cancel: &CancellationToken,
) -> ApprovalOutcome {
    // Register before announcing, so an immediate answer is never lost.
    let rx = sess.request_approval(call_id.to_string()).await;
    let event = ExecApprovalRequestEvent {
        model_context: None,
        kind: ExecApprovalKind::Command,
        call_id: call_id.to_string(),
        plugin_id: None,
        script_path: None,
        approval_id: None,
        turn_id: turn_id.to_string(),
        environment_id: None,
        started_at_ms: chrono::Utc::now().timestamp_millis(),
        command: command.to_vec(),
        cwd: cwd.clone().into(),
        reason,
        network_approval_context: None,
        proposed_execpolicy_amendment: None,
        proposed_network_policy_amendments: None,
        additional_permissions: None,
        available_decisions: Some(vec![
            ReviewDecision::Approved,
            ReviewDecision::ApprovedForSession,
            ReviewDecision::Abort,
        ]),
        parsed_cmd: parse_command(command),
    };
    sess.send_event(turn_id, EventMsg::ExecApprovalRequest(event))
        .await;
    let decision = tokio::select! {
        decision = rx => decision.unwrap_or(ReviewDecision::Abort),
        _ = cancel.cancelled() => ReviewDecision::Abort,
    };
    match decision {
        ReviewDecision::Approved | ReviewDecision::ApprovedExecpolicyAmendment { .. } => {
            ApprovalOutcome::Approved
        }
        ReviewDecision::ApprovedForSession => ApprovalOutcome::ApprovedForSession,
        ReviewDecision::Denied { rejection } if !rejection.is_empty() => {
            ApprovalOutcome::Rejected(format!("rejected by user: {rejection}"))
        }
        ReviewDecision::Denied { .. } => ApprovalOutcome::Rejected("rejected by user".to_string()),
        ReviewDecision::TimedOut => {
            ApprovalOutcome::Rejected("approval request timed out".to_string())
        }
        ReviewDecision::Abort
        | ReviewDecision::ApprovedMcpPolicyAmendment
        | ReviewDecision::NetworkPolicyAmendment { .. } => {
            ApprovalOutcome::Rejected("aborted by user".to_string())
        }
    }
}
