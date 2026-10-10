// Modified from OpenAI Codex (codex-rs/core/src/agents_md.rs @ 806d973): local, synchronous discovery from the git root (codex-git-utils) to cwd plus `$ETHER_HOME/AGENTS.md`; no executor filesystems, multiple environments, fallback filenames, project-root marker config or trust checks.
//! AGENTS.md discovery and user instruction assembly.
//!
//! The user's `$ETHER_HOME/AGENTS.md` comes first. Then every `AGENTS.override.md` (preferred)
//! or `AGENTS.md` found from the git root down to the working directory, inclusive, in that
//! order. Without a git root only the working directory is searched. Project docs share a
//! 32 KiB budget.

use codex_git_utils::get_git_repo_root;
use std::path::Path;
use std::path::PathBuf;

/// Default filename scanned for AGENTS.md instructions.
pub(crate) const DEFAULT_AGENTS_MD_FILENAME: &str = "AGENTS.md";
/// Preferred local override for AGENTS.md instructions.
const LOCAL_AGENTS_MD_FILENAME: &str = "AGENTS.override.md";
/// When both user and project AGENTS.md docs are present, they are joined with this.
const AGENTS_MD_SEPARATOR: &str = "\n\n--- project-doc ---\n\n";
/// Codex's default `project_doc_max_bytes`.
const PROJECT_DOC_MAX_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LoadedAgentsMd {
    user_instructions: Option<String>,
    project_docs: Vec<String>,
    cwd: String,
}

impl LoadedAgentsMd {
    /// Loads the user and project docs; `None` when there are none.
    pub(crate) fn load(ether_home: &Path, cwd: &Path) -> Option<Self> {
        let user_instructions =
            std::fs::read_to_string(ether_home.join(DEFAULT_AGENTS_MD_FILENAME))
                .ok()
                .filter(|text| !text.trim().is_empty());
        let loaded = Self {
            user_instructions,
            project_docs: read_project_docs(cwd),
            cwd: cwd.display().to_string(),
        };
        (loaded.user_instructions.is_some() || !loaded.project_docs.is_empty()).then_some(loaded)
    }

    /// The directory the instructions apply to; set when project docs were found.
    pub(crate) fn directory(&self) -> Option<String> {
        (!self.project_docs.is_empty()).then(|| self.cwd.clone())
    }

    pub(crate) fn text(&self) -> String {
        let mut output = String::new();
        if let Some(user_instructions) = &self.user_instructions {
            output.push_str(user_instructions);
        }
        for (index, doc) in self.project_docs.iter().enumerate() {
            if !output.is_empty() {
                output.push_str(if index == 0 {
                    AGENTS_MD_SEPARATOR
                } else {
                    "\n\n"
                });
            }
            output.push_str(doc);
        }
        output
    }
}

fn read_project_docs(cwd: &Path) -> Vec<String> {
    let mut remaining = PROJECT_DOC_MAX_BYTES;
    let mut docs = Vec::new();
    for path in agents_md_paths(cwd) {
        if remaining == 0 {
            break;
        }
        let mut data = match std::fs::read(&path) {
            Ok(data) => data,
            Err(err) => {
                tracing::warn!("cannot read {}: {err}", path.display());
                continue;
            }
        };
        if data.len() > remaining {
            tracing::warn!(
                path = %path.display(),
                remaining_bytes = remaining,
                "project doc exceeds remaining budget; truncating"
            );
            data.truncate(remaining);
        }
        let text = String::from_utf8_lossy(&data).to_string();
        if !text.trim().is_empty() {
            remaining = remaining.saturating_sub(data.len());
            docs.push(text);
        }
    }
    docs
}

/// Discovers AGENTS.md files from the git root to `cwd`, inclusive. Symlinks are allowed.
fn agents_md_paths(cwd: &Path) -> Vec<PathBuf> {
    let mut search_dirs = Vec::new();
    match get_git_repo_root(cwd) {
        Some(root) => {
            let mut cursor = Some(cwd);
            while let Some(dir) = cursor {
                search_dirs.push(dir.to_path_buf());
                if dir == root {
                    break;
                }
                cursor = dir.parent();
            }
            search_dirs.reverse();
        }
        None => search_dirs.push(cwd.to_path_buf()),
    }
    search_dirs
        .into_iter()
        .filter_map(|dir| {
            [LOCAL_AGENTS_MD_FILENAME, DEFAULT_AGENTS_MD_FILENAME]
                .into_iter()
                .map(|name| dir.join(name))
                .find(|candidate| candidate.is_file())
        })
        .collect()
}
