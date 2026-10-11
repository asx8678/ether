// Modified from OpenAI Codex (codex-rs/execpolicy/src/executable_name.rs @ 806d973): removed Windows-only code.
use std::path::Path;

pub(crate) fn executable_lookup_key(raw: &str) -> String {
    #[cfg(not(windows))]
    {
        raw.to_string()
    }
}

pub(crate) fn executable_path_lookup_key(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(executable_lookup_key)
}
