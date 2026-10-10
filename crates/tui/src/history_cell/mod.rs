// Modified from OpenAI Codex (codex-rs/tui/src/history_cell/mod.rs @ 806d973): the `HistoryCell` trait and shared helpers with the cells ether renders; warnings picker, activity groups and disclosure, MCP, search, session, hook and other product cells are removed.
//! Renderable units of conversation history.

use crate::diff_model::FileChange;
use crate::diff_render::create_diff_summary;
use crate::exec_cell::CommandOutput;
use crate::exec_cell::OutputLinesParams;
use crate::exec_cell::TOOL_CALL_MAX_LINES;
use crate::exec_cell::output_lines;
use crate::render::renderable::Renderable;
use crate::style::proposed_plan_style;
use crate::terminal_hyperlinks::HyperlinkLine;
use crate::terminal_hyperlinks::HyperlinkParagraph;
use crate::terminal_hyperlinks::plain_hyperlink_lines;
use crate::terminal_hyperlinks::prefix_hyperlink_lines;
use crate::terminal_hyperlinks::visible_lines;
use crate::ui_consts::LIVE_PREFIX_COLS;
use crate::wrapping::RtOptions;
use crate::wrapping::adaptive_wrap_line;
use crate::wrapping::adaptive_wrap_lines;
use codex_protocol::models::local_image_label_text;
use codex_protocol::plan_tool::PlanItemArg;
use codex_protocol::plan_tool::StepStatus;
use codex_protocol::plan_tool::UpdatePlanArgs;
use codex_protocol::user_input::TextElement;
use codex_utils_absolute_path::AbsolutePathBuf;
use ratatui::prelude::*;
use ratatui::style::Style;
use ratatui::style::Styled;
use ratatui::style::Stylize;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use std::any::Any;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use url::Url;

const RAW_DIFF_SUMMARY_WIDTH: usize = 10_000;

mod activity_details;
mod activity_group;
pub(crate) mod activity_preview;
mod base;
mod markdown_render_cache;
mod messages;
mod notices;
mod patches;
mod plans;

pub(crate) use activity_details::ActivityDetails;
pub(crate) use activity_group::ActivityGroup;
pub(crate) use activity_preview::ActivityDisclosure;
pub(crate) use base::*;
pub(crate) use messages::*;
pub(crate) use notices::*;
pub(crate) use patches::*;
pub(crate) use plans::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HistoryRenderMode {
    Rich,
    Raw,
}

pub(crate) fn raw_lines_from_source(source: &str) -> Vec<Line<'static>> {
    if source.is_empty() {
        return Vec::new();
    }

    let mut parts = source.split('\n').collect::<Vec<_>>();
    if source.ends_with('\n') {
        parts.pop();
    }

    parts
        .into_iter()
        .map(|line| Line::from(line.to_string()))
        .collect()
}

pub(crate) fn plain_lines(lines: impl IntoIterator<Item = Line<'static>>) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .map(|line| {
            let text = line
                .spans
                .into_iter()
                .map(|span| span.content.into_owned())
                .collect::<String>();
            Line::from(text)
        })
        .collect()
}

/// A single renderable unit of conversation history.
///
/// Each cell produces logical `Line`s and reports how many viewport rows those lines occupy at a
/// given terminal width. The default height implementations use `Paragraph::wrap` to account for
/// lines that overflow the viewport width (e.g. long URLs kept intact by adaptive wrapping).
pub(crate) trait HistoryCell: std::fmt::Debug + Send + Sync + Any {
    /// Retain reasoning inside an activity group, or return it for normal insertion.
    fn append_reasoning(&mut self, cell: Box<dyn HistoryCell>) -> Result<(), Box<dyn HistoryCell>> {
        Err(cell)
    }

    /// Original Markdown for semantic copy targets, independent of presentation.
    fn copy_source(&self) -> Option<&str> {
        None
    }

    /// Returns the logical lines for the main chat viewport.
    fn display_lines(&self, width: u16) -> Vec<Line<'static>>;

    /// Returns copy-friendly plain logical lines for raw scrollback mode.
    fn raw_lines(&self) -> Vec<Line<'static>>;

    /// Raw live presentation may omit transcript-only diagnostics without losing their source.
    fn live_raw_lines(&self) -> Vec<Line<'static>> {
        self.raw_lines()
    }

    /// Returns rich visible lines plus terminal hyperlink metadata.
    fn display_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        plain_hyperlink_lines(self.display_lines(width))
    }

    fn display_lines_for_mode(&self, width: u16, mode: HistoryRenderMode) -> Vec<Line<'static>> {
        match mode {
            HistoryRenderMode::Rich => visible_lines(self.display_hyperlink_lines(width)),
            HistoryRenderMode::Raw => self.live_raw_lines(),
        }
    }

    fn display_hyperlink_lines_for_mode(
        &self,
        width: u16,
        mode: HistoryRenderMode,
    ) -> Vec<HyperlinkLine> {
        match mode {
            HistoryRenderMode::Rich => self.display_hyperlink_lines(width),
            HistoryRenderMode::Raw => plain_hyperlink_lines(self.live_raw_lines()),
        }
    }

    /// Returns the number of viewport rows needed to render this cell.
    fn desired_height(&self, width: u16) -> u16 {
        self.desired_height_for_mode(width, HistoryRenderMode::Rich)
    }

    fn desired_height_for_mode(&self, width: u16, mode: HistoryRenderMode) -> u16 {
        Paragraph::new(Text::from(self.display_lines_for_mode(width, mode)))
            .wrap(Wrap { trim: false })
            .line_count(width)
            .try_into()
            .unwrap_or(0)
    }

    /// Returns lines for a transcript view. Defaults to `display_lines`.
    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        self.display_lines(width)
    }

    /// Returns transcript lines plus terminal hyperlink metadata.
    fn transcript_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        plain_hyperlink_lines(self.transcript_lines(width))
    }

    /// Whether the cached transcript layout remains valid across later frames.
    fn has_stable_transcript_height(&self) -> bool {
        true
    }

    fn is_stream_continuation(&self) -> bool {
        false
    }

    /// A coarse "animation tick" when output is time-dependent; `None` when stable.
    fn transcript_animation_tick(&self) -> Option<u64> {
        None
    }
}

impl Renderable for Box<dyn HistoryCell> {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        let hyperlink_lines = self.display_hyperlink_lines(area.width);
        let paragraph = HyperlinkParagraph::new(&hyperlink_lines, Style::default());
        let y = if area.height == 0 {
            0
        } else {
            let overflow = paragraph
                .line_count(area.width)
                .saturating_sub(usize::from(area.height));
            u16::try_from(overflow).unwrap_or(u16::MAX)
        };
        // Active-cell content can reflow during resize/stream updates; clear stale glyphs first.
        Clear.render(area, buf);
        paragraph.scroll(y).render(area, buf);
    }
    fn desired_height(&self, width: u16) -> u16 {
        HistoryCell::desired_height(self.as_ref(), width)
    }
}

impl dyn HistoryCell {
    pub(crate) fn as_any(&self) -> &dyn Any {
        self
    }

    pub(crate) fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
