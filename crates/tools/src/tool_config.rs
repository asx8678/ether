// Modified from OpenAI Codex (codex-rs/tools/src/tool_config.rs @ 806d973): removed unit tests; takes a bool instead of codex-features.
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::TUI_VISIBLE_COLLABORATION_MODES;

/// `default_mode_request_user_input` replaces Codex's `Feature::DefaultModeRequestUserInput` flag.
pub fn request_user_input_available_modes(default_mode_request_user_input: bool) -> Vec<ModeKind> {
    TUI_VISIBLE_COLLABORATION_MODES
        .into_iter()
        .filter(|mode| {
            mode.allows_request_user_input()
                || (default_mode_request_user_input && *mode == ModeKind::Default)
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolEnvironmentMode {
    None,
    Single,
    Multiple,
}

impl ToolEnvironmentMode {
    pub fn from_count(count: usize) -> Self {
        match count {
            0 => Self::None,
            1 => Self::Single,
            _ => Self::Multiple,
        }
    }

    pub fn has_environment(self) -> bool {
        !matches!(self, Self::None)
    }
}
