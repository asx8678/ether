// New in ether: terminal setup and the inline viewport, after Codex's `Tui` (codex-rs/tui/src/tui.rs @ 806d973) without alternate screen, job control, notifications or pets.
//! The TUI draws in an inline viewport at the bottom of the terminal. Finished history is
//! written above it into the terminal's own scrollback, so it stays selectable and scrollable
//! after ether exits.

// Lifted from Codex's `tui/` with their full API; ether uses part of it.
#[allow(dead_code, unused_imports)]
mod frame_rate_limiter;
#[allow(dead_code, unused_imports)]
pub(crate) mod frame_requester;
#[allow(dead_code, unused_imports)]
mod scrollback;

#[allow(unused_imports)]
pub(crate) use frame_requester::FrameRequester;

use crate::custom_terminal;
use crate::insert_history::HistoryLineWrapPolicy;
use crate::insert_history::insert_history_hyperlink_lines_with_mode_and_wrap_policy;
use crate::terminal_detection::terminal_info;
use crate::terminal_hyperlinks::HyperlinkLine;
use crossterm::cursor::MoveTo;
use crossterm::event::DisableBracketedPaste;
use crossterm::event::EnableBracketedPaste;
use crossterm::execute;
use crossterm::terminal::Clear;
use crossterm::terminal::ClearType;
use crossterm::terminal::disable_raw_mode;
use crossterm::terminal::enable_raw_mode;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::layout::Size;
use scrollback::ScrollbackStrategy;
use std::io;
use std::io::Write;
use std::io::Stdout;
use std::io::stdout;

pub(crate) type Terminal = custom_terminal::Terminal<CrosstermBackend<Stdout>>;

/// Puts the terminal into raw mode with bracketed paste, and restores it when dropped (also on
/// panic, through the hook installed by [`Tui::init`]).
pub(crate) struct Tui {
    terminal: Terminal,
    scrollback: ScrollbackStrategy,
    pending_history: Vec<Vec<HyperlinkLine>>,
    /// Escape sequences (image uploads) to write before the next history insertion.
    pending_raw: Vec<u8>,
}

pub(crate) fn restore() {
    let _ = execute!(stdout(), DisableBracketedPaste);
    let _ = disable_raw_mode();
}

impl Tui {
    pub(crate) fn init() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(stdout(), EnableBracketedPaste)?;
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            previous_hook(info);
        }));
        let terminal = custom_terminal::Terminal::with_options(CrosstermBackend::new(stdout()))?;
        Ok(Self {
            terminal,
            scrollback: ScrollbackStrategy::detect(&terminal_info()),
            pending_history: Vec::new(),
            pending_raw: Vec::new(),
        })
    }

    /// Queues lines to be written above the viewport on the next draw.
    pub(crate) fn insert_history(&mut self, lines: Vec<HyperlinkLine>) {
        if !lines.is_empty() {
            self.pending_history.push(lines);
        }
    }

    /// Queues raw bytes, such as a kitty image upload, for the next draw.
    pub(crate) fn write_raw(&mut self, bytes: &[u8]) {
        self.pending_raw.extend_from_slice(bytes);
    }

    pub(crate) fn screen_size(&mut self) -> io::Result<Size> {
        let (width, height) = crossterm::terminal::size()?;
        Ok(Size::new(width, height))
    }

    /// Draws a viewport of `height` rows at the bottom of the screen, after writing any queued
    /// history above it.
    pub(crate) fn draw(
        &mut self,
        height: u16,
        draw_fn: impl FnOnce(&mut custom_terminal::Frame),
    ) -> io::Result<()> {
        let screen_size = self.screen_size()?;
        let terminal = &mut self.terminal;
        let mut area = terminal.viewport_area;
        area.height = height.min(screen_size.height);
        area.width = screen_size.width;
        if area.bottom() > screen_size.height {
            // The viewport grew past the bottom: scroll history up to make room.
            self.scrollback.grow_viewport(
                terminal,
                area.top(),
                screen_size,
                area.bottom() - screen_size.height,
            )?;
            area.y = screen_size.height - area.height;
        }
        if area != terminal.viewport_area {
            let clear_from = if terminal.viewport_area.is_empty() {
                area.as_position()
            } else {
                terminal.viewport_area.as_position()
            };
            terminal.clear_after_position(clear_from)?;
            terminal.set_viewport_area(area);
        }
        if !self.pending_raw.is_empty() {
            let raw = std::mem::take(&mut self.pending_raw);
            let writer = terminal.backend_mut();
            writer.write_all(&raw)?;
            writer.flush()?;
        }
        for lines in std::mem::take(&mut self.pending_history) {
            let wrap_policy = HistoryLineWrapPolicy::PreWrap;
            let mode = self.scrollback.history_insertion_mode(wrap_policy);
            insert_history_hyperlink_lines_with_mode_and_wrap_policy(
                terminal,
                &lines,
                mode,
                wrap_policy,
                screen_size,
            )?;
        }
        terminal.draw_with_size(screen_size, draw_fn)
    }

    /// Clears the viewport and leaves the cursor below the history, for a clean exit.
    pub(crate) fn finish(&mut self) -> io::Result<()> {
        let area: Rect = self.terminal.viewport_area;
        let writer = self.terminal.backend_mut();
        execute!(writer, MoveTo(0, area.y), Clear(ClearType::FromCursorDown))
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        restore();
    }
}
