// Modified from OpenAI Codex (codex-rs/tui/src/width.rs @ 806d973): removed unit tests.
//! Terminal display-width helpers and guards for fixed prefix columns.
//!
//! Several rendering paths reserve a fixed number of columns for bullets,
//! gutters, or labels before laying out content.  When the terminal is very
//! narrow, those reserved columns can consume the entire width, leaving zero
//! or negative space for content.
//!
//! The display-width helpers match Ratatui's terminal-cell semantics while retaining `usize`
//! precision for long lines. The guards centralise subtraction and enforce a strict-positive
//! contract: they return `Some(n)` where `n > 0`, or `None` when no usable
//! content width remains.  Callers treat `None` as "render prefix-only
//! fallback" rather than attempting wrapped rendering at zero width, which
//! would produce empty or unstable output.

use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

/// Returns the display width Ratatui uses for terminal text without its `u16` limit.
pub(crate) fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
        + text
            .chars()
            .filter(|ch| matches!(ch, '\u{FF9E}' | '\u{FF9F}'))
            .count()
}

/// Returns a scalar's terminal width, treating halfwidth sound marks as visible cells.
pub(crate) fn char_width(ch: char) -> usize {
    if matches!(ch, '\u{FF9E}' | '\u{FF9F}') {
        1
    } else {
        UnicodeWidthChar::width(ch).unwrap_or(0)
    }
}

/// Returns usable content width after reserving fixed columns.
///
/// Guarantees a strict positive width (`Some(n)` where `n > 0`) or `None` when
/// the reserved columns consume the full width.
///
/// Treat `None` as "render prefix-only fallback". Coercing it to `0` and still
/// attempting wrapped rendering often produces empty or unstable output at very
/// narrow terminal widths.
pub(crate) fn usable_content_width(total_width: usize, reserved_cols: usize) -> Option<usize> {
    total_width
        .checked_sub(reserved_cols)
        .filter(|remaining| *remaining > 0)
}

/// `u16` convenience wrapper around [`usable_content_width`].
///
/// This keeps width math at callsites that receive terminal dimensions as
/// `u16` while preserving the same `None` contract for exhausted width.
pub(crate) fn usable_content_width_u16(total_width: u16, reserved_cols: u16) -> Option<usize> {
    usable_content_width(usize::from(total_width), usize::from(reserved_cols))
}
