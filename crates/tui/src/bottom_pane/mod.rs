// New in ether: the bottom pane (composer and overlays).
pub(crate) mod paste_burst;
pub(crate) mod textarea;

use crate::key_hint;
use crate::style::secondary_text_style;
use ratatui::style::Styled;
use ratatui::text::Line;

/// Renders `key label · key label` footer hints. Lifted from Codex's `bottom_pane/footer.rs`.
pub(crate) fn footer_hint_items_line(items: &[(String, String)]) -> Line<'static> {
    let mut spans = Vec::with_capacity(items.len() * 4);
    for (idx, (key, label)) in items.iter().enumerate() {
        if idx > 0 {
            spans.push(" · ".set_style(secondary_text_style()));
        }
        spans.extend(key_hint::key_label_spans(key));
        spans.push(format!(" {label}").set_style(secondary_text_style()));
    }
    Line::from(spans)
}
