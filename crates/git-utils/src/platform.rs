// Modified from OpenAI Codex (codex-rs/git-utils/src/platform.rs @ 806d973): removed Windows-only code.
use std::path::Path;

use crate::GitToolingError;

#[cfg(unix)]
pub fn create_symlink(
    _source: &Path,
    link_target: &Path,
    destination: &Path,
) -> Result<(), GitToolingError> {
    use std::os::unix::fs::symlink;

    symlink(link_target, destination)?;
    Ok(())
}

#[cfg(not(any(unix, windows)))]
compile_error!("codex-git symlink support is only implemented for Unix and Windows");
