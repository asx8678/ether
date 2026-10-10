// Modified from OpenAI Codex (codex-rs/core/src/context/world_state/permissions.rs and codex-rs/prompts/src/permissions_instructions.rs @ 806d973): sandbox and approval text from the bundled templates only, compared by rendered text; no exec-policy prefixes, granular or auto-review approvals, request_permissions, denied reads or message catalogs.
//! The `<permissions instructions>` developer section: the sandbox mode, network access, the
//! approval policy and the writable roots.

use super::WorldStateSection;
use crate::config::EngineConfig;
use codex_context_fragments::ContextualUserFragment;
use codex_protocol::config_types::SandboxMode;
use codex_protocol::models::ContentItemKind;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::NetworkAccess;

const DANGER_FULL_ACCESS_TEMPLATE: &str =
    include_str!("../../templates/permissions/danger_full_access.md");
const WORKSPACE_WRITE_TEMPLATE: &str =
    include_str!("../../templates/permissions/workspace_write.md");
const READ_ONLY_TEMPLATE: &str = include_str!("../../templates/permissions/read_only.md");
const NEVER: &str = include_str!("../../templates/permissions/never.md");
const UNLESS_TRUSTED: &str = include_str!("../../templates/permissions/unless_trusted.md");
const ON_REQUEST: &str = include_str!("../../templates/permissions/on_request.md");
const GRANULAR_INTRO: &str = "# Approval Requests\n\nApproval policy is `granular`. Categories set to `false` are automatically rejected instead of prompting the user.";

/// Permission instructions currently visible to the model.
#[derive(Clone, Debug)]
pub(crate) struct PermissionsState {
    instructions: PermissionsInstructions,
}

impl PermissionsState {
    pub(crate) fn new(config: &EngineConfig) -> Self {
        Self {
            instructions: PermissionsInstructions::new(
                config.sandbox_mode,
                config.approval_policy,
                config.cwd.as_path(),
            ),
        }
    }
}

impl WorldStateSection for PermissionsState {
    /// The rendered instructions; any difference re-sends them whole, as Codex does.
    type Snapshot = String;

    fn render_diff(
        &self,
        previous: Option<&Self::Snapshot>,
    ) -> (Self::Snapshot, Vec<Box<dyn ContextualUserFragment + Send>>) {
        let current = self.instructions.text.clone();
        if previous == Some(&current) {
            return (current, Vec::new());
        }
        (current, vec![Box::new(self.instructions.clone())])
    }
}

/// Developer fragment describing the active sandbox and approval policy.
#[derive(Clone, Debug)]
struct PermissionsInstructions {
    text: String,
}

impl PermissionsInstructions {
    fn new(
        sandbox_mode: SandboxMode,
        approval_policy: AskForApproval,
        cwd: &std::path::Path,
    ) -> Self {
        let profile = permission_profile(sandbox_mode);
        let network_access = if profile.network_sandbox_policy().is_enabled() {
            NetworkAccess::Enabled
        } else {
            NetworkAccess::Restricted
        };
        let file_system_policy = profile.file_system_sandbox_policy();
        let mut writable_roots = if file_system_policy.has_full_disk_write_access() {
            Vec::new()
        } else {
            file_system_policy.get_writable_roots_with_cwd(cwd)
        };
        writable_roots.sort_by(|left, right| left.root.as_path().cmp(right.root.as_path()));
        let writable_roots: Vec<String> = writable_roots
            .iter()
            .map(|root| root.root.to_string_lossy().into_owned())
            .collect();

        let mut text = String::new();
        append_section(&mut text, &sandbox_text(sandbox_mode, network_access));
        append_section(&mut text, &approval_text(approval_policy));
        if let Some(writable_roots) = writable_roots_text(&writable_roots) {
            append_section(&mut text, &writable_roots);
        }
        if !text.ends_with('\n') {
            text.push('\n');
        }
        Self { text }
    }
}

impl ContextualUserFragment for PermissionsInstructions {
    fn role(&self) -> &'static str {
        "developer"
    }

    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("permissions.instructions".to_string())
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("<permissions instructions>", "</permissions instructions>")
    }

    fn body(&self) -> String {
        self.text.clone()
    }
}

/// The permission profile a sandbox mode stands for.
fn permission_profile(sandbox_mode: SandboxMode) -> PermissionProfile {
    match sandbox_mode {
        SandboxMode::ReadOnly => PermissionProfile::read_only(),
        SandboxMode::WorkspaceWrite => PermissionProfile::workspace_write(),
        SandboxMode::DangerFullAccess => PermissionProfile::Disabled,
    }
}

fn append_section(text: &mut String, section: &str) {
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(section);
}

fn sandbox_text(mode: SandboxMode, network_access: NetworkAccess) -> String {
    let template = match mode {
        SandboxMode::DangerFullAccess => DANGER_FULL_ACCESS_TEMPLATE,
        SandboxMode::WorkspaceWrite => WORKSPACE_WRITE_TEMPLATE,
        SandboxMode::ReadOnly => READ_ONLY_TEMPLATE,
    };
    template
        .trim_end()
        .replace("{{ network_access }}", &network_access.to_string())
}

fn approval_text(approval_policy: AskForApproval) -> String {
    match approval_policy {
        AskForApproval::Never => NEVER.to_string(),
        AskForApproval::UnlessTrusted => UNLESS_TRUSTED.to_string(),
        AskForApproval::OnRequest => ON_REQUEST.to_string(),
        AskForApproval::Granular(_) => GRANULAR_INTRO.to_string(),
    }
}

fn writable_roots_text(writable_roots: &[String]) -> Option<String> {
    if writable_roots.is_empty() {
        return None;
    }

    let roots_list: Vec<String> = writable_roots
        .iter()
        .map(|root| format!("`{root}`"))
        .collect();
    Some(if roots_list.len() == 1 {
        format!(" The writable root is {}.", roots_list[0])
    } else {
        format!(" The writable roots are {}.", roots_list.join(", "))
    })
}
