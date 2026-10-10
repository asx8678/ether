// Modified from OpenAI Codex (codex-rs/core/src/safety.rs @ 806d973): patch checks only; the permission profile comes from ether's `SandboxMode`; no executor-managed route, Windows sandbox level or platform-sandbox probe (ether applies patches in-process after this check, decision C2).
//! Decides whether a patch may be applied without asking the user.

use codex_apply_patch::ApplyPatchAction;
use codex_apply_patch::ApplyPatchFileChange;
use codex_protocol::config_types::SandboxMode;
use codex_protocol::models::PermissionProfile;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::FileSystemSandboxPolicyContext;
use codex_protocol::permissions::LocalFileSystemPolicyMatcher;
use codex_protocol::protocol::AskForApproval;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_path_uri::PathUri;
use std::io;
use std::path::PathBuf;

const PATCH_REJECTED_OUTSIDE_PROJECT_REASON: &str =
    "writing outside of the project; rejected by user approval settings";
const PATCH_REJECTED_READ_ONLY_REASON: &str =
    "writing is blocked by read-only sandbox; rejected by user approval settings";

#[derive(Debug, PartialEq)]
pub(crate) enum SafetyCheck {
    AutoApprove,
    AskUser,
    Reject { reason: String },
}

/// The permission profile a sandbox mode stands for.
pub(crate) fn permission_profile_for(sandbox_mode: SandboxMode) -> PermissionProfile {
    match sandbox_mode {
        SandboxMode::ReadOnly => PermissionProfile::read_only(),
        SandboxMode::WorkspaceWrite => PermissionProfile::workspace_write(),
        SandboxMode::DangerFullAccess => PermissionProfile::Disabled,
    }
}

/// Write-permission facts for one session cwd: the filesystem policy and the paths it is
/// resolved against.
pub(crate) struct PatchPermissions {
    profile: PermissionProfile,
    policy: FileSystemSandboxPolicy,
    cwd: PathUri,
    workspace_roots: Vec<PathUri>,
    user_home_dir: Option<PathUri>,
    temporary_directories: Vec<PathUri>,
}

impl PatchPermissions {
    pub(crate) fn new(sandbox_mode: SandboxMode, cwd: &AbsolutePathBuf) -> Self {
        let profile = permission_profile_for(sandbox_mode);
        let policy = profile.file_system_sandbox_policy();
        let cwd = PathUri::from_abs_path(cwd);
        Self {
            profile,
            policy,
            workspace_roots: vec![cwd.clone()],
            cwd,
            user_home_dir: PathUri::from_host_native_path("~").ok(),
            temporary_directories: local_temporary_directories(),
        }
    }

    fn context(&self) -> FileSystemSandboxPolicyContext<'_> {
        FileSystemSandboxPolicyContext {
            cwd: &self.cwd,
            workspace_roots: &self.workspace_roots,
            user_home_dir: self.user_home_dir.as_ref(),
            temporary_directories: Some(&self.temporary_directories),
        }
    }
}

fn local_temporary_directories() -> Vec<PathUri> {
    std::env::var_os("TMPDIR")
        .filter(|path| !path.is_empty())
        .and_then(|tmpdir| AbsolutePathBuf::from_absolute_path(PathBuf::from(tmpdir)).ok())
        .map(PathUri::from)
        .into_iter()
        .collect()
}

pub(crate) fn assess_patch_safety(
    action: &ApplyPatchAction,
    policy: AskForApproval,
    permissions: &PatchPermissions,
) -> io::Result<SafetyCheck> {
    if action.is_empty() {
        return Ok(SafetyCheck::Reject {
            reason: "empty patch".to_string(),
        });
    }

    match policy {
        AskForApproval::Never | AskForApproval::OnRequest | AskForApproval::Granular(_) => {
            // Continue to see if this can be auto-approved.
        }
        AskForApproval::UnlessTrusted => {
            return Ok(SafetyCheck::AskUser);
        }
    }

    let rejects_sandbox_approval = matches!(policy, AskForApproval::Never)
        || matches!(
            policy,
            AskForApproval::Granular(granular_config) if !granular_config.sandbox_approval
        );

    let context = permissions.context();
    let matching = permissions.policy.prepare_local_matching(&context)?;
    if is_write_patch_constrained_to_writable_paths(action, permissions, &context, &matching)? {
        Ok(SafetyCheck::AutoApprove)
    } else if rejects_sandbox_approval {
        Ok(SafetyCheck::Reject {
            reason: patch_rejection_reason(permissions, &context).to_string(),
        })
    } else {
        Ok(SafetyCheck::AskUser)
    }
}

fn patch_rejection_reason(
    permissions: &PatchPermissions,
    context: &FileSystemSandboxPolicyContext<'_>,
) -> &'static str {
    let policy = &permissions.policy;
    let has_no_writable_roots = !policy.has_configured_writable_roots(context);
    match permissions.profile {
        PermissionProfile::Managed { .. }
            if !policy.has_full_disk_write_access_with_context(context)
                && has_no_writable_roots =>
        {
            PATCH_REJECTED_READ_ONLY_REASON
        }
        PermissionProfile::Managed { .. }
        | PermissionProfile::Disabled
        | PermissionProfile::External { .. } => PATCH_REJECTED_OUTSIDE_PROJECT_REASON,
    }
}

fn is_write_patch_constrained_to_writable_paths(
    action: &ApplyPatchAction,
    permissions: &PatchPermissions,
    context: &FileSystemSandboxPolicyContext<'_>,
    matching: &LocalFileSystemPolicyMatcher<'_>,
) -> io::Result<bool> {
    // A full-disk policy permits every patch target, so no per-path writable-root check can
    // further constrain the result.
    if permissions
        .policy
        .has_full_disk_write_access_with_context(context)
    {
        return Ok(true);
    }

    for (path, change) in action.changes() {
        match change {
            ApplyPatchFileChange::Add { .. } | ApplyPatchFileChange::Delete { .. } => {
                if !matching.can_write_path(path)? {
                    return Ok(false);
                }
            }
            ApplyPatchFileChange::Update { move_path, .. } => {
                if !matching.can_write_path(path)? {
                    return Ok(false);
                }
                if let Some(dest) = move_path
                    && !matching.can_write_path(dest)?
                {
                    return Ok(false);
                }
            }
        }
    }

    Ok(true)
}
