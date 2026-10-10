// Modified from OpenAI Codex (codex-rs/tui/src/history_cell/notices.rs @ 806d973): info, warning and error notices only; warnings are always shown and are plain prefixed cells.
use super::*;

pub(crate) fn new_warning_event(message: String) -> PrefixedWrappedHistoryCell {
    let style = crate::style::status_style(crate::style::StatusTone::Attention);
    PrefixedWrappedHistoryCell::new(message.set_style(style), "⚠ ".set_style(style), "  ")
}

pub(crate) fn new_info_event(message: String, hint: Option<String>) -> PlainHistoryCell {
    let mut line = vec!["• ".dim(), message.into()];
    if let Some(hint) = hint {
        line.push(" ".into());
        line.push(hint.dark_gray());
    }
    let lines: Vec<Line<'static>> = vec![line.into()];
    PlainHistoryCell { lines }
}

pub(crate) fn new_error_event(message: String) -> PlainHistoryCell {
    let lines: Vec<Line<'static>> = vec![vec![format!("■ {message}").red()].into()];
    PlainHistoryCell { lines }
}
