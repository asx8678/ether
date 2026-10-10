// Modified from OpenAI Codex (codex-rs/tui/src/clipboard_copy.rs @ 806d973): only the copy format and status types the lifted markdown and textarea code name; ether's TUI has no clipboard integration yet.
//! Clipboard types. Copying to the system clipboard is not supported.

use std::sync::Arc;

/// How copied text is offered to the clipboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CopyFormat {
    PlainText,
    Markdown,
    /// Visible selection text is copied literally; only HTML uses the semantic Markdown.
    MarkdownSelection(Arc<str>),
}

/// Outcome of a copy request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CopyStatus {
    Confirmed,
    Unconfirmed,
    Pending(u64),
    Busy,
}

pub(crate) mod worker {
    pub(crate) type CopyResult = Result<super::CopyStatus, String>;
}
