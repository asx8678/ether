// Modified from OpenAI Codex (codex-rs/utils/process/src/lib.rs @ 806d973): removed unit tests; removed Windows-only code.
//! Command construction for background processes that must not allocate a Windows console.
//! Callers retain control of environment, stdio, and process lifetime. Interactive,
//! detached, and private-desktop launches must select their own console policy.

use std::ffi::OsStr;
use std::process::Command;

/// Construct a background command, suppressing console allocation on Windows.
/// Convert with `tokio::process::Command::from` for asynchronous execution.
/// Subsequent Windows `creation_flags` calls must retain `CREATE_NO_WINDOW`.
pub fn background_command(program: impl AsRef<OsStr>) -> Command {
    #[cfg(not(windows))]
    Command::new(program)
}
