// Modified from OpenAI Codex (codex-rs/core/src/exec_policy.rs @ 806d973): rules load synchronously from `$ETHER_HOME/rules/*.rules` only (no codex-config layers, requirements overlays or environment policies); no rule amendments or prefix-rule suggestions; POSIX command parsing only; Granular approval treated like OnRequest.
//! Decides whether a shell command may run, must be approved, or is forbidden, from the
//! user's exec-policy rules with Codex's heuristics as the fallback.

use crate::exec::approval::ExecApprovalRequirement;
use codex_execpolicy::Decision;
use codex_execpolicy::Evaluation;
use codex_execpolicy::MatchOptions;
use codex_execpolicy::Policy;
use codex_execpolicy::PolicyParser;
use codex_execpolicy::RuleMatch;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::permissions::FileSystemSandboxKind;
use codex_protocol::protocol::AskForApproval;
use codex_shell_command::bash::parse_shell_lc_plain_commands;
use codex_shell_command::is_dangerous_command::DangerousCommandMatch;
use codex_shell_command::is_dangerous_command::dangerous_command_match;
use std::path::Path;
use std::path::PathBuf;

pub(crate) const PROMPT_CONFLICT_REASON: &str =
    "approval required by policy, but AskForApproval is set to Never";
const RULES_DIR_NAME: &str = "rules";
const RULE_EXTENSION: &str = "rules";

/// Loads every `*.rules` file in `<ether_home>/rules`, in name order. A missing directory is an
/// empty policy.
pub(crate) fn load_exec_policy(ether_home: &Path) -> Result<Policy, String> {
    let dir = ether_home.join(RULES_DIR_NAME);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Policy::empty()),
        Err(err) => {
            return Err(format!(
                "failed to read rules from {}: {err}",
                dir.display()
            ));
        }
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext == RULE_EXTENSION)
        })
        .collect();
    paths.sort();
    let mut parser = PolicyParser::new();
    for path in &paths {
        let contents = std::fs::read_to_string(path)
            .map_err(|err| format!("failed to read rules file {}: {err}", path.display()))?;
        let identifier = path.to_string_lossy().to_string();
        parser
            .parse(&identifier, &contents)
            .map_err(|err| format!("failed to parse rules file {identifier}: {err}"))?;
    }
    Ok(parser.build())
}

fn is_policy_match(rule_match: &RuleMatch) -> bool {
    match rule_match {
        RuleMatch::PrefixRuleMatch { .. } => true,
        RuleMatch::HeuristicsRuleMatch { .. } => false,
    }
}

/// Returns a rejection reason when `approval_policy` disallows surfacing a prompt.
pub(crate) fn prompt_is_rejected_by_policy(
    approval_policy: AskForApproval,
) -> Option<&'static str> {
    match approval_policy {
        AskForApproval::Never => Some(PROMPT_CONFLICT_REASON),
        AskForApproval::OnRequest | AskForApproval::UnlessTrusted | AskForApproval::Granular(_) => {
            None
        }
    }
}

/// Evaluates `command` against the rules, falling back to the heuristics for unmatched parts.
pub(crate) fn exec_approval_requirement(
    policy: &Policy,
    command: &[String],
    approval_policy: AskForApproval,
    permission_profile: &PermissionProfile,
    sandbox_permissions: SandboxPermissions,
) -> ExecApprovalRequirement {
    let commands = commands_for_exec_policy(command);
    let fallback = |cmd: &[String]| {
        render_decision_for_unmatched_command(
            cmd,
            approval_policy,
            permission_profile,
            sandbox_permissions,
        )
    };
    let match_options = MatchOptions {
        resolve_host_executables: true,
    };
    let evaluation = policy.check_multiple_with_options(commands.iter(), &fallback, &match_options);

    match evaluation.decision {
        Decision::Forbidden => ExecApprovalRequirement::Forbidden {
            reason: derive_forbidden_reason(
                command,
                &evaluation,
                dangerous_command_match_for_heuristics(&evaluation, Decision::Forbidden),
            ),
        },
        Decision::Prompt => {
            let prompt_is_rule = evaluation.matched_rules.iter().any(|rule_match| {
                is_policy_match(rule_match) && rule_match.decision() == Decision::Prompt
            });
            match prompt_is_rejected_by_policy(approval_policy) {
                Some(reason) if prompt_is_rule => ExecApprovalRequirement::Forbidden {
                    reason: reason.to_string(),
                },
                Some(reason) => ExecApprovalRequirement::Forbidden {
                    reason: derive_rejected_prompt_reason(
                        reason,
                        dangerous_command_match_for_heuristics(&evaluation, Decision::Prompt),
                    ),
                },
                None => ExecApprovalRequirement::NeedsApproval {
                    reason: derive_prompt_reason(command, &evaluation),
                },
            }
        }
        Decision::Allow => ExecApprovalRequirement::Skip {
            // Bypass sandbox only when every parsed command segment is
            // explicitly allowed by execpolicy.
            bypass_sandbox: commands.iter().all(|command| {
                policy
                    .matches_for_command_with_options(
                        command,
                        /*heuristics_fallback*/ None,
                        &match_options,
                    )
                    .iter()
                    .any(|rule_match| {
                        is_policy_match(rule_match) && rule_match.decision() == Decision::Allow
                    })
            }),
        },
    }
}

/// Extract DangerousCommandMatch from an Evaluation
fn dangerous_command_match_for_heuristics(
    evaluation: &Evaluation,
    decision: Decision,
) -> Option<DangerousCommandMatch> {
    evaluation
        .matched_rules
        .iter()
        .find_map(|rule_match| match rule_match {
            RuleMatch::HeuristicsRuleMatch {
                command,
                decision: matched_decision,
            } if *matched_decision == decision => dangerous_command_match(command),
            _ => None,
        })
}

/// If a command is not matched by any execpolicy rule, derive a [`Decision`].
fn render_decision_for_unmatched_command(
    command: &[String],
    approval_policy: AskForApproval,
    permission_profile: &PermissionProfile,
    sandbox_permissions: SandboxPermissions,
) -> Decision {
    // If the command is flagged as dangerous, never allow it to run without approval. Prefer to
    // prompt, but forbid it when prompts are disabled.
    if dangerous_command_match(command).is_some() {
        return match approval_policy {
            AskForApproval::Never => Decision::Forbidden,
            AskForApproval::OnRequest
            | AskForApproval::UnlessTrusted
            | AskForApproval::Granular(_) => Decision::Prompt,
        };
    }

    match approval_policy {
        // Run the command, relying on the sandbox for protection.
        AskForApproval::Never => Decision::Allow,
        // Untrusted projects require approval for every command that is not explicitly
        // allowed by an exec policy rule.
        AskForApproval::UnlessTrusted => Decision::Prompt,
        AskForApproval::OnRequest | AskForApproval::Granular(_) => {
            match permission_profile.file_system_sandbox_policy().kind {
                // The user asked us to "just run" commands in their unrestricted environment.
                FileSystemSandboxKind::Unrestricted | FileSystemSandboxKind::ExternalSandbox => {
                    Decision::Allow
                }
                // In restricted sandboxes, only prompt when the model asks for escalation;
                // otherwise the sandbox enforces the restrictions.
                FileSystemSandboxKind::Restricted => {
                    if sandbox_permissions.requests_sandbox_override() {
                        Decision::Prompt
                    } else {
                        Decision::Allow
                    }
                }
            }
        }
    }
}

fn commands_for_exec_policy(command: &[String]) -> Vec<Vec<String>> {
    if let Some(commands) = parse_shell_lc_plain_commands(command)
        && !commands.is_empty()
    {
        return commands;
    }
    vec![command.to_vec()]
}

fn derive_prompt_reason(command_args: &[String], evaluation: &Evaluation) -> Option<String> {
    let command = render_shlex_command(command_args);

    let most_specific_prompt = evaluation
        .matched_rules
        .iter()
        .filter_map(|rule_match| match rule_match {
            RuleMatch::PrefixRuleMatch {
                matched_prefix,
                decision: Decision::Prompt,
                justification,
                ..
            } => Some((matched_prefix.len(), justification.as_deref())),
            _ => None,
        })
        .max_by_key(|(matched_prefix_len, _)| *matched_prefix_len);

    match most_specific_prompt {
        Some((_matched_prefix_len, Some(justification))) => {
            Some(format!("`{command}` requires approval: {justification}"))
        }
        Some((_matched_prefix_len, None)) => {
            Some(format!("`{command}` requires approval by policy"))
        }
        None => None,
    }
}

fn render_shlex_command(args: &[String]) -> String {
    shlex::try_join(args.iter().map(String::as_str)).unwrap_or_else(|_| args.join(" "))
}

/// Derive a string explaining why the command was forbidden.
fn derive_forbidden_reason(
    command_args: &[String],
    evaluation: &Evaluation,
    dangerous_command_match: Option<DangerousCommandMatch>,
) -> String {
    let command = render_shlex_command(command_args);

    let most_specific_forbidden = evaluation
        .matched_rules
        .iter()
        .filter_map(|rule_match| match rule_match {
            RuleMatch::PrefixRuleMatch {
                matched_prefix,
                decision: Decision::Forbidden,
                justification,
                ..
            } => Some((matched_prefix, justification.as_deref())),
            _ => None,
        })
        .max_by_key(|(matched_prefix, _)| matched_prefix.len());

    match most_specific_forbidden {
        Some((_matched_prefix, Some(justification))) => {
            format!("`{command}` rejected: {justification}")
        }
        Some((matched_prefix, None)) => {
            let prefix = render_shlex_command(matched_prefix);
            format!("`{command}` rejected: policy forbids commands starting with `{prefix}`")
        }
        None => {
            if let Some(dangerous_command_match) = dangerous_command_match {
                let reason = dangerous_command_rejection_reason(dangerous_command_match);
                format!("`{command}` rejected: {reason}")
            } else {
                format!("`{command}` rejected: blocked by policy")
            }
        }
    }
}

fn derive_rejected_prompt_reason(
    fallback_reason: &str,
    dangerous_command_match: Option<DangerousCommandMatch>,
) -> String {
    match dangerous_command_match {
        Some(dangerous_command_match @ DangerousCommandMatch::ForcedRm) => {
            dangerous_command_rejection_reason(dangerous_command_match).to_string()
        }
        Some(DangerousCommandMatch::Other) | None => fallback_reason.to_string(),
    }
}

fn dangerous_command_rejection_reason(
    dangerous_command_match: DangerousCommandMatch,
) -> &'static str {
    match dangerous_command_match {
        DangerousCommandMatch::ForcedRm => {
            "rm -f style commands are not permitted. Use a safer approach"
        }
        DangerousCommandMatch::Other => "blocked by policy",
    }
}
