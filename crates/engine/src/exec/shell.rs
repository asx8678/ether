// Modified from OpenAI Codex (codex-rs/core/src/shell.rs @ 806d973): POSIX shells only (no PowerShell/cmd); exec-server shell info, login PATH prepends and shell snapshots removed.
//! The user's shell, used to run `exec_command` scripts.

use codex_shell_command::shell_detect::DetectedShell;
use codex_shell_command::shell_detect::ShellType;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Shell {
    pub(crate) shell_type: ShellType,
    pub(crate) shell_path: PathBuf,
}

impl Shell {
    /// Returns the argv that runs `command` in this shell.
    pub(crate) fn derive_exec_args(&self, command: &str, use_login_shell: bool) -> Vec<String> {
        match self.shell_type {
            ShellType::Zsh | ShellType::Bash | ShellType::Sh => {
                let arg = if use_login_shell { "-lc" } else { "-c" };
                vec![
                    self.shell_path.to_string_lossy().to_string(),
                    arg.to_string(),
                    command.to_string(),
                ]
            }
        }
    }
}

impl From<DetectedShell> for Shell {
    fn from(detected: DetectedShell) -> Self {
        Self {
            shell_type: detected.shell_type,
            shell_path: detected.shell_path,
        }
    }
}

pub(crate) fn get_shell_by_model_provided_path(shell_path: &PathBuf) -> Shell {
    codex_shell_command::shell_detect::get_shell_by_model_provided_path(shell_path).into()
}

pub(crate) fn default_user_shell() -> Shell {
    codex_shell_command::shell_detect::default_user_shell().into()
}
