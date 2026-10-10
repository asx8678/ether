// Modified from OpenAI Codex (codex-rs/shell-command/src/command_safety/mod.rs @ 806d973): removed unit tests.
// Keep the PowerShell subprocess parser available as a test oracle, but do not
// compile it into production command classification.
mod powershell_tree_sitter;

pub mod is_dangerous_command;
pub(crate) use powershell_tree_sitter::try_parse_powershell_commands;
