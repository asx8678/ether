// New in ether: prompt history across sessions, in the role of Codex's message-history crate.
//! Submitted prompts are appended to `$ETHER_HOME/history.jsonl` (one `{"ts", "text"}` object per
//! line, mode 0600) and the newest ones are loaded at start. Up and Down walk through them; the
//! text being written when browsing starts comes back after the newest entry.

use serde::Deserialize;
use serde::Serialize;
use std::io::BufRead;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;

const FILE_NAME: &str = "history.jsonl";
/// Entries kept in memory for browsing.
const MAX_ENTRIES: usize = 1000;

#[derive(Serialize, Deserialize)]
struct Entry {
    ts: i64,
    text: String,
}

#[derive(Debug, Default)]
pub(crate) struct PromptHistory {
    path: Option<PathBuf>,
    entries: Vec<String>,
    /// The entry being shown while browsing, and the draft it replaced.
    index: Option<usize>,
    draft: String,
}

impl PromptHistory {
    /// Loads the newest entries from `ether_home`; a missing or unreadable file is empty.
    pub(crate) fn load(ether_home: &Path) -> Self {
        let path = ether_home.join(FILE_NAME);
        let mut entries: Vec<String> = std::fs::File::open(&path)
            .map(|file| {
                std::io::BufReader::new(file)
                    .lines()
                    .map_while(Result::ok)
                    .filter_map(|line| serde_json::from_str::<Entry>(&line).ok())
                    .map(|entry| entry.text)
                    .collect()
            })
            .unwrap_or_default();
        if entries.len() > MAX_ENTRIES {
            entries.drain(..entries.len() - MAX_ENTRIES);
        }
        Self {
            path: Some(path),
            entries,
            ..Self::default()
        }
    }

    /// Records a submitted prompt (not twice in a row) and ends browsing.
    pub(crate) fn record(&mut self, text: &str) {
        self.reset();
        if text.trim().is_empty() || self.entries.last().is_some_and(|last| last == text) {
            return;
        }
        self.entries.push(text.to_string());
        if self.entries.len() > MAX_ENTRIES {
            self.entries.remove(0);
        }
        if let Some(path) = &self.path {
            let line = serde_json::to_string(&Entry {
                ts: chrono::Utc::now().timestamp(),
                text: text.to_string(),
            });
            let written = line.map_err(std::io::Error::other).and_then(|line| {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .mode(0o600)
                    .open(path)?;
                writeln!(file, "{line}")
            });
            if let Err(err) = written {
                tracing::warn!("cannot save prompt history to {}: {err}", path.display());
            }
        }
    }

    /// Whether Up/Down are walking through the history.
    pub(crate) fn browsing(&self) -> bool {
        self.index.is_some()
    }

    /// The entry before the one shown; `current` is kept as the draft when browsing starts.
    pub(crate) fn previous(&mut self, current: &str) -> Option<String> {
        let index = match self.index {
            None if self.entries.is_empty() => return None,
            None => {
                self.draft = current.to_string();
                self.entries.len() - 1
            }
            Some(0) => return None,
            Some(index) => index - 1,
        };
        self.index = Some(index);
        self.entries.get(index).cloned()
    }

    /// The entry after the one shown, or the draft after the newest entry.
    pub(crate) fn next(&mut self) -> Option<String> {
        let index = self.index?;
        if index + 1 < self.entries.len() {
            self.index = Some(index + 1);
            self.entries.get(index + 1).cloned()
        } else {
            self.index = None;
            Some(std::mem::take(&mut self.draft))
        }
    }

    /// Stops browsing (the shown text becomes the draft).
    pub(crate) fn reset(&mut self) {
        self.index = None;
        self.draft.clear();
    }
}
