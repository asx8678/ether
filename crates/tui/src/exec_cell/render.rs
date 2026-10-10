// Modified from OpenAI Codex (exec_cell/render.rs @ 806d973): removed unit tests.
//! Command previews and full transcripts, retaining logical text through display wrapping.

use std::time::Instant;

use super::model::CommandOutput;
use super::model::ExecCall;
use super::model::ExecCell;
use crate::exec_command::strip_bash_lc_and_escape;
use crate::history_cell::ActivityDisclosure;
use crate::history_cell::HistoryCell;
use crate::history_cell::HistoryRenderMode;
use crate::history_cell::plain_lines;
use crate::motion::MotionMode;
use crate::motion::ReducedMotionIndicator;
use crate::motion::activity_indicator;
use crate::render::highlight::highlight_bash_to_lines;
use crate::render::line_utils::line_to_static;
use crate::style::accent_color;
use crate::terminal_hyperlinks::HyperlinkLine;
use crate::terminal_hyperlinks::LogicalLineSource;
use crate::terminal_hyperlinks::adaptive_wrap_hyperlink_lines;
use crate::terminal_hyperlinks::plain_hyperlink_lines;
use crate::terminal_hyperlinks::prefix_hyperlink_lines;
use crate::terminal_hyperlinks::remap_source_wrapped_line;
use crate::terminal_hyperlinks::visible_lines;
use crate::tool_output::tool_output_hyperlink_preview;
use crate::ui_consts::transcript_hint;
use crate::wrapping::RtOptions;
use crate::wrapping::adaptive_wrap_line_with_source;
use codex_ansi_escape::ansi_escape_line;
use codex_protocol::parse_command::ParsedCommand;
use codex_protocol::protocol::ExecCommandSource;
use codex_shell_command::bash::extract_bash_command;
use itertools::Itertools;
use ratatui::prelude::*;
use ratatui::style::Modifier;
use ratatui::style::Stylize;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Wrap;
use textwrap::WordSplitter;
use unicode_width::UnicodeWidthStr;

pub(crate) const TOOL_CALL_MAX_LINES: usize = 5;
const USER_SHELL_TOOL_CALL_MAX_LINES: usize = 50;
const MAX_INTERACTION_PREVIEW_CHARS: usize = 80;

pub(crate) struct OutputLinesParams {
    pub(crate) line_limit: usize,
    pub(crate) only_err: bool,
    pub(crate) include_angle_pipe: bool,
    pub(crate) include_prefix: bool,
}

struct CommandDisplay {
    lines: Vec<HyperlinkLine>,
    hidden_details: bool,
}

pub(crate) fn new_active_exec_command(
    call_id: String,
    command: Vec<String>,
    parsed: Vec<ParsedCommand>,
    source: ExecCommandSource,
    interaction_input: Option<String>,
    animations_enabled: bool,
) -> ExecCell {
    ExecCell::new(
        ExecCall {
            call_id,
            command,
            parsed,
            output: None,
            source,
            start_time: Some(Instant::now()),
            duration: None,
            interaction_input,
        },
        animations_enabled,
    )
}

fn format_unified_exec_interaction(command: &[String], input: Option<&str>) -> String {
    let command_display = if let Some((_, script)) = extract_bash_command(command) {
        script.to_string()
    } else {
        command.join(" ")
    };
    match input {
        Some(data) if !data.is_empty() => {
            let preview = summarize_interaction_input(data);
            format!("Interacted with `{command_display}`, sent `{preview}`")
        }
        _ => format!("Waited for `{command_display}`"),
    }
}

fn summarize_interaction_input(input: &str) -> String {
    let single_line = input.replace('\n', "\\n");
    let sanitized = single_line.replace('`', "\\`");
    if sanitized.chars().count() <= MAX_INTERACTION_PREVIEW_CHARS {
        return sanitized;
    }

    let mut preview = String::new();
    for ch in sanitized.chars().take(MAX_INTERACTION_PREVIEW_CHARS) {
        preview.push(ch);
    }
    preview.push_str("...");
    preview
}

#[derive(Clone)]
pub(crate) struct OutputLines {
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) omitted: Option<usize>,
}

pub(crate) fn output_lines(
    output: Option<&CommandOutput>,
    params: OutputLinesParams,
) -> OutputLines {
    let OutputLinesParams {
        line_limit,
        only_err,
        include_angle_pipe,
        include_prefix,
    } = params;
    let output = match output {
        Some(output) if only_err && output.exit_code == 0 => {
            return OutputLines {
                lines: Vec::new(),
                omitted: None,
            };
        }
        Some(output) => output,
        None => {
            return OutputLines {
                lines: Vec::new(),
                omitted: None,
            };
        }
    };

    let (total, retained) = output.line_counts();
    let mut out: Vec<Line<'static>> = Vec::new();

    let head_end = total.min(line_limit).min(retained);
    for (i, raw) in output.lines().take(head_end).enumerate() {
        let mut line = dimmed_output_line(raw.as_ref());
        let prefix = if !include_prefix {
            ""
        } else if i == 0 && include_angle_pipe {
            "  └ "
        } else {
            "    "
        };
        line.spans.insert(/*index*/ 0, prefix.dim());
        out.push(line);
    }

    let tail_len = total
        .saturating_sub(head_end)
        .min(line_limit)
        .min(retained.saturating_sub(head_end));
    let omitted = total.saturating_sub(head_end + tail_len);
    let omitted = (omitted > 0).then_some(omitted);
    if let Some(omitted) = omitted {
        out.push(ExecCell::output_ellipsis_line(omitted));
    }

    let tail = output.lines().rev().take(tail_len).collect_vec();
    for raw in tail.into_iter().rev() {
        let mut line = dimmed_output_line(raw.as_ref());
        if include_prefix {
            line.spans.insert(/*index*/ 0, "    ".dim());
        }
        out.push(line);
    }

    OutputLines {
        lines: out,
        omitted,
    }
}

fn dimmed_output_line(raw: &str) -> Line<'static> {
    let mut line = ansi_escape_line(raw);
    for span in &mut line.spans {
        span.style = span.style.add_modifier(Modifier::DIM);
    }
    line
}

fn output_preview_lines(output: &CommandOutput, width: usize) -> Vec<HyperlinkLine> {
    let (total, _) = output.line_counts();
    tool_output_hyperlink_preview(
        output.lines().map(|raw| dimmed_output_line(raw.as_ref())),
        width,
        total,
    )
}

fn activity_marker(start_time: Option<Instant>, animations_enabled: bool) -> Span<'static> {
    activity_indicator(
        start_time,
        MotionMode::from_animations_enabled(animations_enabled),
        ReducedMotionIndicator::StaticBullet,
    )
    .unwrap_or_else(|| "•".dim())
}

impl HistoryCell for ExecCell {
    fn append_reasoning(&mut self, cell: Box<dyn HistoryCell>) -> Result<(), Box<dyn HistoryCell>> {
        if self.is_exploring_cell() {
            self.group.push_detail(std::sync::Arc::from(cell));
            Ok(())
        } else {
            Err(cell)
        }
    }

    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        visible_lines(self.display_hyperlink_lines(width))
    }

    fn display_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        if self.is_exploring_cell() {
            self.exploring_display_lines(width)
        } else {
            self.command_display_lines(width)
        }
    }

    fn transcript_lines(&self, width: u16) -> Vec<Line<'static>> {
        visible_lines(self.transcript_hyperlink_lines(width))
    }

    fn transcript_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.detailed_hyperlink_lines(width, HistoryRenderMode::Rich)
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        plain_lines(visible_lines(
            self.detailed_hyperlink_lines(u16::MAX, HistoryRenderMode::Raw),
        ))
    }
}

impl ExecCell {
    fn output_ellipsis_text(omitted: usize) -> String {
        let noun = if omitted == 1 { "line" } else { "lines" };
        format!("… +{omitted} {noun} ({})", transcript_hint())
    }

    fn output_ellipsis_line(omitted: usize) -> Line<'static> {
        Line::from(vec![Self::output_ellipsis_text(omitted).dim()])
    }

    fn exploring_display_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        let mut out: Vec<HyperlinkLine> = Vec::new();
        out.push(
            Line::from(vec![
                if self.is_active() {
                    activity_marker(self.active_start_time(), self.animations_enabled())
                } else {
                    "•".dim()
                },
                " ".into(),
                if self.is_active() {
                    "Exploring".bold()
                } else {
                    "Explored".bold()
                },
            ])
            .into(),
        );

        let mut calls = self.group.calls.as_slice();
        let mut out_indented = Vec::new();
        let nonzero_exit = |call: &ExecCall| {
            call.duration
                .and(call.output.as_ref())
                .map(|output| output.exit_code)
                .filter(|code| *code != 0)
        };
        while let Some((call, remaining)) = calls.split_first() {
            let exit_code = nonzero_exit(call);
            let reads_only = call
                .parsed
                .iter()
                .all(|parsed| matches!(parsed, ParsedCommand::Read { .. }));
            let group_len = if reads_only && exit_code.is_none() {
                1 + remaining
                    .iter()
                    .take_while(|next| {
                        nonzero_exit(next).is_none()
                            && next
                                .parsed
                                .iter()
                                .all(|parsed| matches!(parsed, ParsedCommand::Read { .. }))
                    })
                    .count()
            } else {
                1
            };
            let (group, remaining) = calls.split_at(group_len);
            calls = remaining;

            let call_lines: Vec<(&str, Vec<Span<'static>>)> = if reads_only {
                let names = group
                    .iter()
                    .flat_map(|call| &call.parsed)
                    .map(|parsed| match parsed {
                        ParsedCommand::Read { name, .. } => name.clone(),
                        _ => unreachable!(),
                    })
                    .unique();
                vec![(
                    "Read",
                    Itertools::intersperse(names.into_iter().map(Into::into), ", ".dim()).collect(),
                )]
            } else {
                let mut lines = Vec::new();
                for parsed in &call.parsed {
                    match parsed {
                        ParsedCommand::Read { name, .. } => {
                            lines.push(("Read", vec![name.clone().into()]));
                        }
                        ParsedCommand::ListFiles { cmd, path } => {
                            lines.push(("List", vec![path.clone().unwrap_or(cmd.clone()).into()]));
                        }
                        ParsedCommand::Search { cmd, query, path } => {
                            let spans = match (query, path) {
                                (Some(q), Some(p)) => {
                                    vec![q.clone().into(), " in ".dim(), p.clone().into()]
                                }
                                (Some(q), None) => vec![q.clone().into()],
                                _ => vec![cmd.clone().into()],
                            };
                            lines.push(("Search", spans));
                        }
                        ParsedCommand::Unknown { cmd } => {
                            lines.push(("Run", vec![cmd.clone().into()]));
                        }
                    }
                }
                lines
            };

            let line_count = call_lines.len();
            for (index, (title, mut line)) in call_lines.into_iter().enumerate() {
                if let Some(code) = exit_code
                    && index + 1 == line_count
                {
                    // A compound command has one exit code, not an outcome for each parsed action.
                    let status = if call.parsed.len() > 1 {
                        format!(" (command exit {code})")
                    } else {
                        format!(" (exit {code})")
                    };
                    // Search exit 1 can mean no matches; report the code without calling it a failure.
                    line.push(
                        if code == 1
                            && call
                                .parsed
                                .iter()
                                .any(|p| matches!(p, ParsedCommand::Search { .. }))
                        {
                            status.dim()
                        } else {
                            status.red()
                        },
                    );
                }
                let line = Line::from(line);
                let initial_indent = Line::from(vec![title.fg(accent_color()), " ".into()]);
                let subsequent_indent = " ".repeat(initial_indent.width()).into();
                let wrapped = adaptive_wrap_hyperlink_lines(
                    &[line.into()],
                    RtOptions::new(width as usize)
                        .initial_indent(initial_indent)
                        .subsequent_indent(subsequent_indent),
                );
                out_indented.extend(wrapped);
            }
        }

        out.extend(prefix_hyperlink_lines(
            out_indented,
            "  └ ".dim(),
            "    ".into(),
        ));
        out
    }

    fn command_display_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        self.command_display_lines_with_hidden_details(width).lines
    }

    fn command_display_lines_with_hidden_details(&self, width: u16) -> CommandDisplay {
        let [call] = &self.group.calls.as_slice() else {
            panic!("Expected exactly one call in a command display cell");
        };
        let layout = EXEC_DISPLAY_LAYOUT;
        let success = call
            .duration
            .and_then(|_| call.output.as_ref().map(|o| o.exit_code == 0));
        let bullet = match success {
            Some(true) => "•".green().bold(),
            Some(false) => "•".red().bold(),
            None => activity_marker(call.start_time, self.animations_enabled()),
        };
        let is_interaction = call.is_unified_exec_interaction();
        let title = if is_interaction {
            ""
        } else if self.is_active() {
            "Running"
        } else if call.is_user_shell_command() {
            "You ran"
        } else {
            "Ran"
        };

        let header_line = if is_interaction {
            Line::from(vec![bullet.clone(), " ".into()])
        } else {
            Line::from(vec![bullet.clone(), " ".into(), title.bold(), " ".into()])
        };
        let header_prefix_width = header_line.width();
        let mut header = HyperlinkLine::from(header_line.clone());

        let cmd_display = if call.is_unified_exec_interaction() {
            format_unified_exec_interaction(&call.command, call.interaction_input.as_deref())
        } else {
            strip_bash_lc_and_escape(&call.command)
        };
        let highlighted_lines = highlight_bash_to_lines(&cmd_display);

        let continuation_wrap_width = layout.command_continuation.wrap_width(width);
        let continuation_opts =
            RtOptions::new(continuation_wrap_width).word_splitter(WordSplitter::NoHyphenation);

        let mut continuation_lines: Vec<HyperlinkLine> = Vec::new();

        if let Some((first, rest)) = highlighted_lines.split_first() {
            let available_first_width = (width as usize)
                .saturating_sub(header_prefix_width)
                .max(/*other*/ 1);
            let first_opts =
                RtOptions::new(available_first_width).word_splitter(WordSplitter::NoHyphenation);

            let source = LogicalLineSource::from_line(first);
            let first_wrapped = adaptive_wrap_line_with_source(first, first_opts);
            let mut first_wrapped_iter = first_wrapped.into_iter();
            if let Some(first_segment) = first_wrapped_iter.next() {
                let mut header_source = source.clone();
                header_source.range = first_segment.range;
                header_source.prefix_bytes = first_segment.prefix_bytes
                    + header_line
                        .spans
                        .iter()
                        .map(|span| span.content.len())
                        .sum::<usize>();
                header_source.continuation_indent =
                    Line::from(layout.command_continuation.subsequent_prefix);
                let mut line = line_to_static(&first_segment.line);
                line.spans.splice(0..0, header_line.spans.clone());
                header = HyperlinkLine::new(line);
                header.source = Some(header_source);
            }
            let mut command = HyperlinkLine::new(first.clone());
            command.source = Some(source);
            continuation_lines.extend(remap_source_wrapped_line(
                &command,
                first_wrapped_iter.collect(),
            ));

            for line in rest {
                continuation_lines.extend(adaptive_wrap_hyperlink_lines(
                    &[line.clone().into()],
                    continuation_opts.clone(),
                ));
            }
        }

        let mut lines: Vec<HyperlinkLine> = vec![header];

        let mut hidden_details = continuation_lines.len() > layout.command_continuation_max_lines;
        let continuation_lines = Self::limit_lines_from_start(
            &continuation_lines,
            layout.command_continuation_max_lines,
        );
        if !continuation_lines.is_empty() {
            lines.extend(prefix_hyperlink_lines(
                continuation_lines,
                Span::from(layout.command_continuation.initial_prefix).dim(),
                Span::from(layout.command_continuation.subsequent_prefix).dim(),
            ));
        }

        if let Some(output) = call.output.as_ref() {
            if !call.is_user_shell_command() {
                let preview = output_preview_lines(output, layout.output_block.wrap_width(width));
                let preview = if preview.is_empty() && !call.is_unified_exec_interaction() {
                    vec![Line::from("(no output)".dim()).into()]
                } else {
                    preview
                };
                // Only the omission hint lacks source provenance; visible output rows retain it.
                hidden_details |=
                    preview.iter().any(|line| line.source.is_none()) && output.line_counts().0 > 0;
                lines.extend(prefix_hyperlink_lines(
                    preview,
                    Span::from(layout.output_block.initial_prefix).dim(),
                    Span::from(layout.output_block.subsequent_prefix),
                ));
                return CommandDisplay {
                    lines,
                    hidden_details,
                };
            }
            let raw_output = output_lines(
                Some(output),
                OutputLinesParams {
                    line_limit: USER_SHELL_TOOL_CALL_MAX_LINES,
                    only_err: false,
                    include_angle_pipe: false,
                    include_prefix: false,
                },
            );

            if raw_output.lines.is_empty() {
                if !call.is_unified_exec_interaction() {
                    lines.extend(prefix_hyperlink_lines(
                        vec![Line::from("(no output)".dim()).into()],
                        Span::from(layout.output_block.initial_prefix).dim(),
                        Span::from(layout.output_block.subsequent_prefix),
                    ));
                }
            } else {
                // Wrap first so that truncation is applied to on-screen lines
                // rather than logical lines. This ensures that a small number
                // of very long lines cannot flood the viewport.

                let output_wrap_width = layout.output_block.wrap_width(width);
                let output_opts =
                    RtOptions::new(output_wrap_width).word_splitter(WordSplitter::NoHyphenation);
                let wrapped_output = adaptive_wrap_hyperlink_lines(
                    &plain_hyperlink_lines(raw_output.lines),
                    output_opts,
                );

                let prefixed_output = prefix_hyperlink_lines(
                    wrapped_output,
                    Span::from(layout.output_block.initial_prefix).dim(),
                    Span::from(layout.output_block.subsequent_prefix),
                );
                let trimmed_output = Self::truncate_lines_middle(
                    &prefixed_output,
                    USER_SHELL_TOOL_CALL_MAX_LINES,
                    width,
                    raw_output.omitted,
                    Some(Line::from(
                        Span::from(layout.output_block.subsequent_prefix).dim(),
                    )),
                );
                // Source-backed output rows retain provenance through prefixing and wrapping.
                // Middle truncation inserts an unbacked omission row only when output is hidden.
                hidden_details |= trimmed_output.iter().any(|line| line.source.is_none());

                if !trimmed_output.is_empty() {
                    lines.extend(trimmed_output);
                }
            }
        }

        CommandDisplay {
            lines,
            hidden_details,
        }
    }

    fn limit_lines_from_start(lines: &[HyperlinkLine], keep: usize) -> Vec<HyperlinkLine> {
        if lines.len() <= keep {
            return lines.to_vec();
        }
        if keep == 0 {
            return vec![Self::ellipsis_line(lines.len()).into()];
        }

        let mut out: Vec<HyperlinkLine> = lines[..keep].to_vec();
        out.push(Self::ellipsis_line(lines.len() - keep).into());
        out
    }

    /// Truncates a list of lines to fit within `max_rows` viewport rows,
    /// keeping a head portion and a tail portion with an ellipsis line
    /// in between.
    ///
    /// `max_rows` is measured in viewport rows (the actual space a line
    /// occupies after `Paragraph::wrap`), not logical lines. Each line's
    /// row cost is computed via `Paragraph::line_count` at the given
    /// `width`. This ensures that a single logical line containing a
    /// long URL (which wraps to several viewport rows) is properly
    /// accounted for.
    ///
    /// The ellipsis message reports the number of omitted *lines*
    /// (logical, not rows) to keep the count stable across terminal
    /// widths. `omitted_hint` carries forward any previously reported
    /// omitted count (from upstream truncation); `ellipsis_prefix`
    /// prepends the output gutter prefix to the ellipsis line.
    fn truncate_lines_middle(
        lines: &[HyperlinkLine],
        max_rows: usize,
        width: u16,
        omitted_hint: Option<usize>,
        ellipsis_prefix: Option<Line<'static>>,
    ) -> Vec<HyperlinkLine> {
        let width = width.max(/*other*/ 1);
        if max_rows == 0 {
            return Vec::new();
        }
        let line_rows: Vec<usize> = lines
            .iter()
            .map(|line| {
                let is_whitespace_only = line
                    .line
                    .spans
                    .iter()
                    .all(|span| span.content.chars().all(char::is_whitespace));
                if is_whitespace_only {
                    line.width().div_ceil(usize::from(width)).max(/*other*/ 1)
                } else {
                    Paragraph::new(Text::from(vec![line.line.clone()]))
                        .wrap(Wrap { trim: false })
                        .line_count(width)
                        .max(/*other*/ 1)
                }
            })
            .collect();
        let total_rows: usize = line_rows.iter().sum();
        if total_rows <= max_rows {
            return lines.to_vec();
        }
        // Reserve space for the transcript hint itself so the returned output
        // still respects the row budget on narrow terminals.
        let estimated_omitted = omitted_hint.unwrap_or(/*default*/ 0)
            + lines
                .len()
                .saturating_sub(usize::from(omitted_hint.is_some()));
        let ellipsis_rows =
            Self::output_ellipsis_row_count(estimated_omitted, width, ellipsis_prefix.as_ref());
        if ellipsis_rows >= max_rows {
            return vec![
                Self::output_ellipsis_line_with_prefix(estimated_omitted, ellipsis_prefix.as_ref())
                    .into(),
            ];
        }

        let available_rows = max_rows - ellipsis_rows;
        let head_budget = available_rows / 2;
        let tail_budget = available_rows - head_budget;
        let mut head_lines: Vec<HyperlinkLine> = Vec::new();
        let mut head_rows = 0usize;
        let mut head_end = 0usize;
        while head_end < lines.len() {
            let line_row_count = line_rows[head_end];
            if head_rows + line_row_count > head_budget {
                break;
            }
            head_rows += line_row_count;
            head_lines.push(lines[head_end].clone());
            head_end += 1;
        }

        let mut tail_lines_reversed: Vec<HyperlinkLine> = Vec::new();
        let mut tail_rows = 0usize;
        let mut tail_start = lines.len();
        while tail_start > head_end {
            let idx = tail_start - 1;
            let line_row_count = line_rows[idx];
            if tail_rows + line_row_count > tail_budget {
                break;
            }
            tail_rows += line_row_count;
            tail_lines_reversed.push(lines[idx].clone());
            tail_start -= 1;
        }

        let mut out = head_lines;
        let base = omitted_hint.unwrap_or(/*default*/ 0);
        let additional = lines
            .len()
            .saturating_sub(out.len() + tail_lines_reversed.len())
            .saturating_sub(usize::from(omitted_hint.is_some()));
        out.push(
            Self::output_ellipsis_line_with_prefix(base + additional, ellipsis_prefix.as_ref())
                .into(),
        );

        out.extend(tail_lines_reversed.into_iter().rev());

        out
    }

    fn ellipsis_line(omitted: usize) -> Line<'static> {
        let noun = if omitted == 1 { "line" } else { "lines" };
        Line::from(vec![format!("… +{omitted} {noun}").dim()])
    }

    fn output_ellipsis_row_count(
        omitted: usize,
        width: u16,
        prefix: Option<&Line<'static>>,
    ) -> usize {
        Paragraph::new(Text::from(vec![Self::output_ellipsis_line_with_prefix(
            omitted, prefix,
        )]))
        .wrap(Wrap { trim: false })
        .line_count(width)
        .max(1)
    }

    /// Builds an output ellipsis line (`… +N lines (ctrl+t to view transcript)`)
    /// with an optional leading prefix so the ellipsis aligns with the output gutter.
    fn output_ellipsis_line_with_prefix(
        omitted: usize,
        prefix: Option<&Line<'static>>,
    ) -> Line<'static> {
        let mut line = prefix.cloned().unwrap_or_default();
        line.push_span(Self::output_ellipsis_text(omitted).dim());
        line
    }
}

#[derive(Clone, Copy)]
struct PrefixedBlock {
    initial_prefix: &'static str,
    subsequent_prefix: &'static str,
}

impl PrefixedBlock {
    const fn new(initial_prefix: &'static str, subsequent_prefix: &'static str) -> Self {
        Self {
            initial_prefix,
            subsequent_prefix,
        }
    }

    fn wrap_width(self, total_width: u16) -> usize {
        let prefix_width = UnicodeWidthStr::width(self.initial_prefix)
            .max(UnicodeWidthStr::width(self.subsequent_prefix));
        usize::from(total_width).saturating_sub(prefix_width).max(1)
    }
}

#[derive(Clone, Copy)]
struct ExecDisplayLayout {
    command_continuation: PrefixedBlock,
    command_continuation_max_lines: usize,
    output_block: PrefixedBlock,
}

impl ExecDisplayLayout {
    const fn new(
        command_continuation: PrefixedBlock,
        command_continuation_max_lines: usize,
        output_block: PrefixedBlock,
    ) -> Self {
        Self {
            command_continuation,
            command_continuation_max_lines,
            output_block,
        }
    }
}

const EXEC_DISPLAY_LAYOUT: ExecDisplayLayout = ExecDisplayLayout::new(
    PrefixedBlock::new("  │ ", "  │ "),
    /*command_continuation_max_lines*/ 2,
    PrefixedBlock::new("  └ ", "    "),
);
