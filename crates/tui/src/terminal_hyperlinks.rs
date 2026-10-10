// Modified from OpenAI Codex (codex-rs/tui/src/terminal_hyperlinks.rs @ 806d973): removed unit tests.
//! Semantic terminal hyperlinks carried separately from visible TUI text.
//!
//! Layout code measures and wraps ordinary ratatui lines. Hyperlink annotations are applied only
//! when text reaches a terminal buffer or scrollback writer so OSC 8 bytes never affect geometry.

mod paragraph;
mod source;

pub(crate) use paragraph::HyperlinkParagraph;
pub(crate) use paragraph::HyperlinkRows;
pub(crate) use paragraph::HyperlinkText;
pub(crate) use source::LineWrapPolicy;
pub(crate) use source::LogicalLineSource;

use std::num::NonZeroU16;
use std::ops::Range;
use std::path::Path;

use ratatui::buffer::Buffer;
use ratatui::buffer::CellDiffOption;
use ratatui::buffer::CellWidth;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use ratatui::widgets::Wrap;
use unicode_segmentation::UnicodeSegmentation;
use url::Url;

use crate::line_truncation::line_width;
use crate::render::line_utils::line_to_borrowed;
use crate::render::line_utils::line_to_static;
use crate::width::display_width;
use crate::wrapping::RtOptions;

// Destinations are repeated in every linked buffer cell. Leave oversized URLs as plain text.
const MAX_HYPERLINK_DESTINATION_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TerminalHyperlink {
    pub(crate) columns: Range<usize>,
    pub(crate) destination: String,
    destination_kind: DestinationKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DestinationKind {
    Web,
    /// A generated visualization or a capability-validated spoken workspace artifact.
    TrustedFile,
}

/// An existing, inert source file proven to remain inside its session workspace.
pub(crate) struct TrustedWorkspaceFile(Url);

const SAFE_WORKSPACE_EXTENSIONS: &[&str] = &[
    "rs", "go", "c", "cc", "cpp", "h", "hpp", "java", "kt", "swift", "toml", "json", "yaml", "yml",
    "md", "txt", "css",
];

impl TrustedWorkspaceFile {
    pub(crate) fn validate(workspace: &Path, candidate: &str) -> Option<Self> {
        if candidate.is_empty()
            || candidate.len() > 512
            || candidate.contains(':')
            || candidate.chars().any(char::is_control)
        {
            return None;
        }
        let normalized = candidate.replace('\\', "/");
        let path = Path::new(&normalized);
        if path.is_absolute()
            || normalized
                .split('/')
                .any(|part| part.is_empty() || part.starts_with('.'))
            || !SAFE_WORKSPACE_EXTENSIONS.contains(&path.extension()?.to_str()?)
        {
            return None;
        }
        let workspace = workspace.canonicalize().ok()?;
        let target = workspace.join(path).canonicalize().ok()?;
        let relative = target.strip_prefix(&workspace).ok()?;
        if !target.metadata().ok()?.is_file()
            || target.extension() != path.extension()
            || relative
                .iter()
                .any(|part| part.to_string_lossy().starts_with('.'))
        {
            return None;
        }
        Url::from_file_path(target).ok().map(Self)
    }
}

impl TerminalHyperlink {
    pub(crate) fn web(columns: Range<usize>, destination: String) -> Self {
        Self {
            columns,
            destination,
            destination_kind: DestinationKind::Web,
        }
    }

    pub(crate) fn retarget_to_trusted_file(&mut self, destination: &Url) {
        // General Markdown never promotes file URLs. Only generated visualization links use this
        // path; spoken workspace artifacts require a separately validated capability.
        debug_assert_eq!(destination.scheme(), "file");
        self.destination = destination.to_string();
        self.destination_kind = DestinationKind::TrustedFile;
    }

    pub(crate) fn trusted_workspace_file(
        columns: Range<usize>,
        file: TrustedWorkspaceFile,
    ) -> Self {
        Self {
            columns,
            destination: file.0.to_string(),
            destination_kind: DestinationKind::TrustedFile,
        }
    }

    pub(crate) fn with_columns(&self, columns: Range<usize>) -> Self {
        Self {
            columns,
            destination: self.destination.clone(),
            destination_kind: self.destination_kind,
        }
    }

    pub(crate) fn terminal_destination(&self) -> Option<String> {
        match self.destination_kind {
            DestinationKind::Web => web_destination(&self.destination),
            DestinationKind::TrustedFile => trusted_file_destination(&self.destination),
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct HyperlinkLine {
    pub(crate) line: Line<'static>,
    pub(crate) hyperlinks: Vec<TerminalHyperlink>,
    pub(crate) source: Option<LogicalLineSource>,
}

// Source provenance is shared layout metadata; omit it from visual diagnostics to avoid
// repeating an entire logical line for every wrapped fragment.
impl std::fmt::Debug for HyperlinkLine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HyperlinkLine")
            .field("line", &self.line)
            .field("hyperlinks", &self.hyperlinks)
            .finish()
    }
}

impl PartialEq for HyperlinkLine {
    fn eq(&self, other: &Self) -> bool {
        self.line == other.line && self.hyperlinks == other.hyperlinks
    }
}

impl Eq for HyperlinkLine {}

/// Cache equality also tracks source whitespace hidden by display wrapping.
pub(crate) fn lines_with_sources_eq(left: &[HyperlinkLine], right: &[HyperlinkLine]) -> bool {
    left == right
        && left
            .iter()
            .map(|line| &line.source)
            .eq(right.iter().map(|line| &line.source))
}

impl HyperlinkLine {
    pub(crate) fn new(line: Line<'static>) -> Self {
        Self {
            line,
            hyperlinks: Vec::new(),
            source: None,
        }
    }

    pub(crate) fn width(&self) -> usize {
        line_width(&self.line)
    }

    pub(crate) fn push_span(&mut self, span: Span<'static>, destination: Option<&str>) {
        self.source = None;
        let start = self.width();
        let end = start + display_width(span.content.as_ref());
        self.line.push_span(span);
        if end > start
            && let Some(destination) = destination.and_then(web_destination)
        {
            self.hyperlinks
                .push(TerminalHyperlink::web(start..end, destination));
        }
    }

    pub(crate) fn style(mut self, style: ratatui::style::Style) -> Self {
        self.line = self.line.style(style);
        if let Some(source) = &mut self.source {
            source.line_style = style;
        }
        self
    }
}

impl From<Line<'static>> for HyperlinkLine {
    fn from(line: Line<'static>) -> Self {
        Self::new(line)
    }
}

impl From<&'static str> for HyperlinkLine {
    fn from(text: &'static str) -> Self {
        Self::new(Line::from(text))
    }
}

impl From<String> for HyperlinkLine {
    fn from(text: String) -> Self {
        Self::new(Line::from(text))
    }
}

pub(crate) fn visible_lines(lines: Vec<HyperlinkLine>) -> Vec<Line<'static>> {
    lines.into_iter().map(|line| line.line).collect()
}

pub(crate) fn visible_lines_ref(lines: &[HyperlinkLine]) -> Vec<Line<'_>> {
    lines
        .iter()
        .map(|line| line_to_borrowed(&line.line))
        .collect()
}

pub(crate) fn plain_hyperlink_lines(lines: Vec<Line<'static>>) -> Vec<HyperlinkLine> {
    lines.into_iter().map(HyperlinkLine::new).collect()
}

pub(crate) fn prefix_hyperlink_lines(
    lines: Vec<HyperlinkLine>,
    initial_prefix: Span<'static>,
    subsequent_prefix: Span<'static>,
) -> Vec<HyperlinkLine> {
    lines
        .into_iter()
        .enumerate()
        .map(|(index, mut line)| {
            let prefix = if index == 0 {
                initial_prefix.clone()
            } else {
                subsequent_prefix.clone()
            };
            let shift = display_width(prefix.content.as_ref());
            let mut source = line
                .source
                .take()
                .unwrap_or_else(|| LogicalLineSource::from_line(&line.line));
            source.prefix_bytes += prefix.content.len();
            source
                .continuation_indent
                .spans
                .insert(/*index*/ 0, subsequent_prefix.clone());
            line.source = Some(source);
            let mut spans = Vec::with_capacity(line.line.spans.len() + 1);
            spans.push(prefix);
            spans.extend(line.line.spans);
            line.line = Line::from(spans).style(line.line.style);
            for hyperlink in &mut line.hyperlinks {
                hyperlink.columns = hyperlink.columns.start + shift..hyperlink.columns.end + shift;
            }
            line
        })
        .collect()
}

/// Retain a known first-line label in copy text without changing the existing wrapping.
///
/// Callers pass the suffix of the first row's synthetic prefix that carries meaning, such as
/// a checkbox or `answer:`. Only that row's shared logical source is updated.
pub(crate) fn retain_initial_prefix(lines: &mut [HyperlinkLine], prefix: &str) {
    let Some(origin) = lines.first().and_then(|line| line.source.clone()) else {
        return;
    };
    debug_assert!(origin.prefix_bytes >= prefix.len());
    let prefix_line = LogicalLineSource::from_line(&lines[0].line)
        .styled_range(origin.prefix_bytes - prefix.len()..origin.prefix_bytes);
    let mut styles = LogicalLineSource::from_line(&prefix_line).styles.to_vec();
    styles.extend(origin.styles.iter().map(|(range, style)| {
        (
            range.start + prefix.len()..range.end + prefix.len(),
            origin.line_style.patch(*style).patch(origin.span_style),
        )
    }));
    let styles = std::sync::Arc::from(styles);
    let text: std::sync::Arc<str> = format!("{prefix}{}", origin.text).into();
    for (index, line) in lines.iter_mut().enumerate() {
        let Some(source) = line
            .source
            .as_mut()
            .filter(|source| std::sync::Arc::ptr_eq(&source.text, &origin.text))
        else {
            continue;
        };
        source.text = std::sync::Arc::clone(&text);
        source.styles = std::sync::Arc::clone(&styles);
        source.line_style = ratatui::style::Style::default();
        source.span_style = ratatui::style::Style::default();
        source.range = source.range.start + prefix.len()..source.range.end + prefix.len();
        if index == 0 {
            source.range.start = 0;
            source.prefix_bytes -= prefix.len();
        }
    }
}

pub(crate) fn adaptive_wrap_hyperlink_lines(
    lines: &[HyperlinkLine],
    options: RtOptions<'static>,
) -> Vec<HyperlinkLine> {
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let options = if index == 0 {
            options.clone()
        } else {
            options
                .clone()
                .initial_indent(options.subsequent_indent.clone())
        };
        let mut source = line.clone();
        source
            .source
            .get_or_insert_with(|| LogicalLineSource::from_line(&line.line))
            .continuation_indent = options.subsequent_indent.clone();
        out.extend(remap_source_wrapped_line(
            &source,
            crate::wrapping::adaptive_wrap_line_with_source(&line.line, options),
        ));
    }
    out
}

pub(crate) fn annotate_web_urls(lines: Vec<Line<'static>>) -> Vec<HyperlinkLine> {
    lines.into_iter().map(annotate_web_urls_in_line).collect()
}

pub(crate) fn annotate_web_urls_in_line(line: Line<'static>) -> HyperlinkLine {
    let text = line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    let mut out = HyperlinkLine::new(line);
    out.hyperlinks = web_links_in_text(&text);
    out
}

/// Project annotations from the exact source slices used by the existing wrapping algorithm.
pub(crate) fn remap_source_wrapped_line(
    source: &HyperlinkLine,
    wrapped: Vec<crate::wrapping::WrappedLine<'_>>,
) -> Vec<HyperlinkLine> {
    let text = line_text(&source.line);
    let mut logical = source
        .source
        .clone()
        .unwrap_or_else(|| LogicalLineSource::from_line(&source.line));
    // Wrapping bakes the row style into each span; mirror it once for all source fragments.
    if logical.line_style != ratatui::style::Style::default() {
        logical.styles = logical
            .styles
            .iter()
            .map(|(range, style)| (range.clone(), logical.line_style.patch(*style)))
            .collect::<Vec<_>>()
            .into();
    }
    let mut source_byte = 0;
    let mut source_column = 0;
    wrapped
        .into_iter()
        .map(|wrapped| {
            let line = line_to_static(&wrapped.line);
            let displayed = line_text(&line);
            let prefix_columns = display_width(&displayed[..wrapped.prefix_bytes]);
            // Some wrapping paths retain a one-past-the-end cursor sentinel. Span slicing already
            // clips that sentinel to the visible line; keep hyperlink and source projection on the
            // same bounded range instead of indexing past the flattened text.
            let range = wrapped.range.start.min(text.len())..wrapped.range.end.min(text.len());
            source_column += display_width(&text[source_byte..range.start]);
            let start = source_column;
            let end = start + display_width(&text[range.clone()]);
            source_byte = range.end;
            source_column = end;
            let hyperlinks = source
                .hyperlinks
                .iter()
                .filter_map(|link| {
                    let first = link.columns.start.max(start);
                    let last = link.columns.end.min(end);
                    (first < last).then(|| {
                        link.with_columns(
                            prefix_columns + first - start..prefix_columns + last - start,
                        )
                    })
                })
                .collect();
            HyperlinkLine {
                line,
                hyperlinks,
                source: Some(logical.wrapped(range, wrapped.prefix_bytes)),
            }
        })
        .collect()
}

/// Re-attach source hyperlink ranges after visible-text wrapping has split a line.
///
/// Link text is matched in display order so a URL split across table rows retains the complete
/// destination on every rendered fragment. Whitespace inserted or removed at line boundaries is
/// ignored while matching; hyperlink destinations themselves are never reconstructed from output.
/// This legacy projection does not infer logical source provenance. New wrapping callers should
/// pass authoritative ranges to [`remap_source_wrapped_line`].
pub(crate) fn remap_wrapped_line(
    source: &HyperlinkLine,
    wrapped: Vec<Line<'static>>,
) -> Vec<HyperlinkLine> {
    let mut out = plain_hyperlink_lines(wrapped);
    if source.hyperlinks.is_empty() && source.source.is_none() {
        return out;
    }
    let source_text = line_text(&source.line);
    if source_text.trim().is_empty() {
        for line in &mut out {
            line.source = source.source.clone();
        }
        return out;
    }
    let mut source_byte = 0usize;
    let mut source_column = 0usize;
    let mut link_index = 0usize;
    for (index, line) in out.iter_mut().enumerate() {
        if index > 0 {
            let trimmed = source_text[source_byte..].trim_start_matches(char::is_whitespace);
            let skipped = source_text[source_byte..].len() - trimmed.len();
            source_column += display_width(&source_text[source_byte..source_byte + skipped]);
            source_byte += skipped;
        }

        let rendered = line_text(&line.line);
        let remaining = &source_text[source_byte..];
        let Some(rendered_start) = longest_suffix_matching_prefix(&rendered, remaining) else {
            continue;
        };
        let mapped = &rendered[rendered_start..];
        line.source = source
            .source
            .as_ref()
            .map(|source| source.wrapped(source_byte..source_byte + mapped.len(), rendered_start));
        let mut output_column = display_width(&rendered[..rendered_start]);
        for grapheme in mapped.graphemes(/*is_extended*/ true) {
            let width = display_width(grapheme);
            while source
                .hyperlinks
                .get(link_index)
                .is_some_and(|link| link.columns.end <= source_column)
            {
                link_index += 1;
            }
            if let Some(link) = source
                .hyperlinks
                .get(link_index)
                .filter(|link| link.columns.contains(&source_column))
            {
                push_link_range(line, output_column..output_column + width, link);
            }
            source_column += width;
            output_column += width;
        }
        source_byte += mapped.len();
    }
    out
}

fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

fn longest_suffix_matching_prefix(rendered: &str, source: &str) -> Option<usize> {
    rendered
        .grapheme_indices(/*is_extended*/ true)
        .map(|(index, _)| index)
        .chain(std::iter::once(rendered.len()))
        .find(|index| source.starts_with(&rendered[*index..]) && *index < rendered.len())
}

fn push_link_range(line: &mut HyperlinkLine, range: Range<usize>, link: &TerminalHyperlink) {
    if range.is_empty() {
        return;
    }
    if let Some(previous) = line.hyperlinks.last_mut()
        && previous.destination == link.destination
        && previous.destination_kind == link.destination_kind
        && previous.columns.end == range.start
    {
        previous.columns.end = range.end;
        return;
    }
    line.hyperlinks.push(link.with_columns(range));
}

pub(crate) fn web_links_in_text(text: &str) -> Vec<TerminalHyperlink> {
    let mut links = Vec::new();
    let mut search_from = 0usize;
    let mut source_byte = 0usize;
    let mut source_column = 0usize;
    for raw_token in text.split_whitespace() {
        let Some(relative_start) = text[search_from..].find(raw_token) else {
            continue;
        };
        let raw_start = search_from + relative_start;
        search_from = raw_start + raw_token.len();
        let trimmed_start = raw_token
            .find(|ch: char| !is_leading_punctuation(ch))
            .unwrap_or(raw_token.len());
        let trimmed_end = trailing_url_end(&raw_token[trimmed_start..]) + trimmed_start;
        if trimmed_start >= trimmed_end {
            continue;
        }
        let candidate = &raw_token[trimmed_start..trimmed_end];
        let Some(destination) = web_destination(candidate) else {
            continue;
        };
        let candidate_start = raw_start + trimmed_start;
        // Measure disjoint prefixes so scanning a draft with many URLs stays linear.
        source_column += display_width(&text[source_byte..candidate_start]);
        source_byte = candidate_start;
        let end = source_column + display_width(candidate);
        links.push(TerminalHyperlink::web(source_column..end, destination));
    }
    links
}

fn is_leading_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | '.' | ';' | '!' | '\'' | '"'
    )
}

fn trailing_url_end(candidate: &str) -> usize {
    // Count delimiter balances once rather than rescanning for every trailing closer.
    let mut balances = [0isize; 4];
    for ch in candidate.chars() {
        match ch {
            '(' => balances[0] += 1,
            ')' => balances[0] -= 1,
            '[' => balances[1] += 1,
            ']' => balances[1] -= 1,
            '{' => balances[2] += 1,
            '}' => balances[2] -= 1,
            '<' => balances[3] += 1,
            '>' => balances[3] -= 1,
            _ => {}
        }
    }
    let mut end = candidate.len();
    while end > 0 {
        let remaining = &candidate[..end];
        let Some(ch) = remaining.chars().next_back() else {
            break;
        };
        let balance = match ch {
            ')' => Some(&mut balances[0]),
            ']' => Some(&mut balances[1]),
            '}' => Some(&mut balances[2]),
            '>' => Some(&mut balances[3]),
            _ => None,
        };
        let trim = if let Some(balance) = balance {
            let unmatched = *balance < 0;
            *balance += 1;
            unmatched
        } else {
            matches!(ch, ',' | '.' | ';' | '!' | '\'' | '"')
                || ch == '?'
                    && match remaining.chars().rev().nth(1) {
                        Some(')') => balances[0] < 0,
                        Some(']') => balances[1] < 0,
                        Some('}') => balances[2] < 0,
                        Some('>') => balances[3] < 0,
                        _ => false,
                    }
        };
        if !trim {
            break;
        }
        end -= ch.len_utf8();
    }
    end
}

pub(crate) fn web_destination(destination: &str) -> Option<String> {
    let safe_destination = sanitized_destination(destination)?;
    let parsed = Url::parse(&safe_destination).ok()?;
    matches!(parsed.scheme(), "http" | "https")
        .then(|| parsed.host_str())
        .flatten()?;
    Some(safe_destination)
}

fn trusted_file_destination(destination: &str) -> Option<String> {
    let safe_destination = sanitized_destination(destination)?;
    let parsed = Url::parse(&safe_destination).ok()?;
    (parsed.scheme() == "file" && parsed.to_file_path().is_ok()).then_some(safe_destination)
}

fn sanitized_destination(destination: &str) -> Option<String> {
    if destination.len() > MAX_HYPERLINK_DESTINATION_BYTES {
        return None;
    }
    Some(destination.chars().filter(|ch| !ch.is_control()).collect())
}

pub(crate) fn osc8_hyperlink(destination: &str, text: &str) -> String {
    let Some(safe_destination) = web_destination(destination) else {
        return text.to_string();
    };
    format!("\x1b]8;;{safe_destination}\x07{text}\x1b]8;;\x07")
}

pub(crate) fn decorate_spans(line: &HyperlinkLine) -> Vec<Span<'static>> {
    if line.hyperlinks.is_empty() {
        return line
            .line
            .spans
            .iter()
            .map(|span| Span {
                style: span.style,
                content: if span.content.contains(char::is_control) {
                    span.content
                        .graphemes(/*is_extended*/ true)
                        .filter(|grapheme| !grapheme.contains(char::is_control))
                        .collect::<String>()
                        .into()
                } else {
                    span.content.clone()
                },
            })
            .collect();
    }

    let mut out = Vec::new();
    let mut column = 0usize;
    let mut link_index = 0usize;
    let mut active_link_index = None;
    let mut active_destination: Option<String> = None;
    for span in &line.line.spans {
        for grapheme in span.content.graphemes(/*is_extended*/ true) {
            let width = display_width(grapheme);
            // Match ratatui's filtering, retaining source columns for semantic links.
            // Only the hyperlink metadata below may introduce terminal escapes.
            if grapheme.contains(char::is_control) {
                column += width;
                continue;
            }
            while line
                .hyperlinks
                .get(link_index)
                .is_some_and(|link| link.columns.end <= column)
            {
                link_index += 1;
            }
            let selected_link_index = line
                .hyperlinks
                .get(link_index)
                .and_then(|link| link.columns.contains(&column).then_some(link_index));
            if active_link_index != selected_link_index {
                if active_destination.is_some() {
                    append_to_last_span(&mut out, "\x1b]8;;\x07");
                }
                active_destination = selected_link_index
                    .and_then(|index| line.hyperlinks[index].terminal_destination());
                if let Some(destination) = active_destination.as_ref() {
                    push_styled_content(
                        &mut out,
                        &format!("\x1b]8;;{destination}\x07"),
                        span.style,
                    );
                }
                active_link_index = selected_link_index;
            }
            push_styled_content(&mut out, grapheme, span.style);
            column += width;
        }
    }
    if active_destination.is_some() {
        append_to_last_span(&mut out, "\x1b]8;;\x07");
    }
    out
}

fn push_styled_content(out: &mut Vec<Span<'static>>, content: &str, style: ratatui::style::Style) {
    if let Some(last) = out.last_mut()
        && last.style == style
    {
        last.content.to_mut().push_str(content);
        return;
    }
    out.push(Span::styled(content.to_string(), style));
}

fn append_to_last_span(out: &mut [Span<'static>], content: &str) {
    if let Some(last) = out.last_mut() {
        last.content.to_mut().push_str(content);
    }
}

pub(crate) fn mark_buffer_hyperlinks(
    buf: &mut Buffer,
    area: Rect,
    lines: &[HyperlinkLine],
    scroll_rows: usize,
) {
    if area.width == 0 || area.height == 0 || lines.iter().all(|line| line.hyperlinks.is_empty()) {
        return;
    }
    let viewport_end = scroll_rows.saturating_add(usize::from(area.height));
    let mut logical_row = 0usize;
    for line in lines {
        if logical_row >= viewport_end {
            break;
        }
        let paragraph =
            Paragraph::new(Text::from(line_to_borrowed(&line.line))).wrap(Wrap { trim: false });
        let rendered_height = paragraph.line_count(area.width).max(/*other*/ 1);
        if line.hyperlinks.is_empty() || logical_row.saturating_add(rendered_height) <= scroll_rows
        {
            logical_row += rendered_height;
            continue;
        }

        let layout_area = Rect::new(
            /*x*/ 0,
            /*y*/ 0,
            area.width,
            u16::try_from(rendered_height).unwrap_or(u16::MAX),
        );
        let mut layout = Buffer::empty(layout_area);
        paragraph.render(layout_area, &mut layout);
        let rendered_lines = (0..layout_area.height)
            .map(|row| {
                let mut trailing_columns = 0usize;
                let text = (0..layout_area.width)
                    .filter_map(|column| {
                        if trailing_columns > 0 {
                            trailing_columns -= 1;
                            return None;
                        }
                        let cell = &layout[(column, row)];
                        if cell.diff_option == CellDiffOption::Skip {
                            return None;
                        }
                        trailing_columns = usize::from(cell.cell_width()).saturating_sub(1);
                        Some(cell.symbol())
                    })
                    .collect::<String>();
                Line::from(text.trim_end().to_string())
            })
            .collect();
        for (row, rendered) in remap_wrapped_line(line, rendered_lines).iter().enumerate() {
            let row = logical_row + row;
            if row < scroll_rows || row >= viewport_end {
                continue;
            }
            for link in &rendered.hyperlinks {
                let Some(destination) = link.terminal_destination() else {
                    continue;
                };
                let mut trailing_columns = 0usize;
                for column in link.columns.clone() {
                    if trailing_columns > 0 {
                        trailing_columns -= 1;
                        continue;
                    }
                    let x = area.x + column as u16;
                    let y = area.y + (row - scroll_rows) as u16;
                    let cell = &mut buf[(x, y)];
                    if cell.diff_option == CellDiffOption::Skip {
                        continue;
                    }
                    trailing_columns = usize::from(cell.cell_width()).saturating_sub(1);
                    let symbol = format!("\x1b]8;;{destination}\x07{}\x1b]8;;\x07", cell.symbol());
                    let width = NonZeroU16::new(cell.cell_width()).unwrap_or(NonZeroU16::MIN);
                    cell.set_symbol(&symbol)
                        .set_diff_option(CellDiffOption::ForcedWidth(width));
                }
            }
        }
        logical_row += rendered_height;
    }
}

pub(crate) fn mark_url_hyperlink(buf: &mut Buffer, area: Rect, destination: &str) {
    mark_matching_cells(buf, area, destination, |cell| {
        cell.fg == Color::Cyan && cell.modifier.contains(Modifier::UNDERLINED)
    });
}

pub(crate) fn mark_underlined_hyperlink(buf: &mut Buffer, area: Rect, destination: &str) {
    mark_matching_cells(buf, area, destination, |cell| {
        cell.modifier.contains(Modifier::UNDERLINED)
    });
}

fn mark_matching_cells(
    buf: &mut Buffer,
    area: Rect,
    destination: &str,
    matches: impl Fn(&ratatui::buffer::Cell) -> bool,
) {
    if web_destination(destination).is_none() {
        return;
    }
    for position in area.positions() {
        let cell = &mut buf[position];
        if cell.diff_option != CellDiffOption::Skip
            && !cell.symbol().trim().is_empty()
            && matches(cell)
        {
            let width = NonZeroU16::new(cell.cell_width()).unwrap_or(NonZeroU16::MIN);
            let symbol = osc8_hyperlink(destination, cell.symbol());
            cell.set_symbol(&symbol)
                .set_diff_option(CellDiffOption::ForcedWidth(width));
        }
    }
}
