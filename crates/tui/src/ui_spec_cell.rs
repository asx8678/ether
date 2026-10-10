// New in ether: the generative UI cell, drawing `render_ui` views.
//! A view ([`Node`] tree) is drawn with ratatui widgets into an off-screen [`Buffer`] sized to
//! fit, and each buffer row becomes one styled [`Line`], so the view goes through the same
//! history path as every other cell. `column`/`row` split the area; `table` is a ratatui
//! `Table`, `gauge` a `LineGauge`, `text` and `markdown` wrapped `Paragraph`s.

use crate::history_cell::HistoryCell;
use crate::markdown_render::render_markdown_text_with_width_and_cwd;
use ether_ui_spec::Cell as SpecCell;
use ether_ui_spec::Color as SpecColor;
use ether_ui_spec::Node;
use ether_ui_spec::gauge_ratio;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;
use ratatui::widgets::Block;
use ratatui::widgets::BorderType;
use ratatui::widgets::LineGauge;
use ratatui::widgets::Padding;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::Widget;
use ratatui::widgets::Wrap;
use std::path::Path;
use std::path::PathBuf;
use unicode_width::UnicodeWidthStr;

/// Longest bar a gauge draws, so a wide terminal does not get a 200-column bar.
const GAUGE_BAR_MAX: u16 = 40;
/// Width of a gauge's value text (`100%`).
const GAUGE_VALUE_WIDTH: usize = 4;
const TABLE_COLUMN_SPACING: u16 = 2;
/// Tallest view drawn; anything below is cut.
const MAX_VIEW_HEIGHT: u16 = 400;

#[derive(Debug)]
pub(crate) struct UiSpecCell {
    view_id: String,
    node: Node,
    cwd: PathBuf,
}

impl UiSpecCell {
    pub(crate) fn new(view_id: String, node: Node, cwd: &Path) -> Self {
        Self {
            view_id,
            node,
            cwd: cwd.to_path_buf(),
        }
    }

    pub(crate) fn view_id(&self) -> &str {
        &self.view_id
    }

    /// Replaces the view, for a later `render_ui` call with the same view id.
    pub(crate) fn set_node(&mut self, node: Node) {
        self.node = node;
    }
}

impl HistoryCell for UiSpecCell {
    fn display_lines(&self, width: u16) -> Vec<Line<'static>> {
        let inner = width.saturating_sub(2).max(1);
        render_lines(&self.node, inner, &self.cwd)
            .into_iter()
            .enumerate()
            .map(|(index, line)| {
                let prefix: Span<'static> = if index == 0 {
                    "• ".dim()
                } else {
                    "  ".into()
                };
                let mut spans = vec![prefix];
                spans.extend(line.spans);
                Line::from(spans)
            })
            .collect()
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        self.display_lines(80)
    }
}

/// Draws `node` at `width` columns and returns its rows as styled lines.
pub(crate) fn render_lines(node: &Node, width: u16, cwd: &Path) -> Vec<Line<'static>> {
    let width = width.max(1);
    let height = height(node, width, cwd).clamp(1, MAX_VIEW_HEIGHT);
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    draw(node, area, &mut buf, 0, cwd);
    buffer_lines(&buf)
}

/// Rows `node` needs at `width`.
fn height(node: &Node, width: u16, cwd: &Path) -> u16 {
    match node {
        Node::Column { children, gap } => {
            let rows: u16 = children
                .iter()
                .map(|child| height(child, width, cwd))
                .fold(0u16, u16::saturating_add);
            let gaps = gap.saturating_mul(children.len().saturating_sub(1) as u16);
            rows.saturating_add(gaps)
        }
        Node::Row { children, gap } => row_areas(Rect::new(0, 0, width, 1), children.len(), *gap)
            .iter()
            .zip(children)
            .map(|(area, child)| height(child, area.width.max(1), cwd))
            .max()
            .unwrap_or(0),
        Node::Text { .. } | Node::Markdown { .. } => paragraph(node, width, cwd)
            .line_count(width)
            .try_into()
            .unwrap_or(u16::MAX),
        Node::Table { rows, .. } => (rows.len() as u16).saturating_add(3),
        Node::Gauge { .. } => 1,
    }
}

/// Draws `node` into `area`. `label_width` aligns the bars of sibling gauges.
fn draw(node: &Node, area: Rect, buf: &mut Buffer, label_width: usize, cwd: &Path) {
    if area.is_empty() {
        return;
    }
    match node {
        Node::Column { children, gap } => {
            let labels = gauge_label_width(children);
            let mut y = area.y;
            for child in children {
                if y >= area.bottom() {
                    break;
                }
                let child_height = height(child, area.width, cwd).min(area.bottom() - y);
                draw(
                    child,
                    Rect::new(area.x, y, area.width, child_height),
                    buf,
                    labels,
                    cwd,
                );
                y = y.saturating_add(child_height).saturating_add(*gap);
            }
        }
        Node::Row { children, gap } => {
            for (child_area, child) in row_areas(area, children.len(), *gap).iter().zip(children) {
                draw(child, *child_area, buf, 0, cwd);
            }
        }
        Node::Text { .. } | Node::Markdown { .. } => {
            paragraph(node, area.width, cwd).render(area, buf);
        }
        Node::Table {
            title,
            headers,
            rows,
        } => draw_table(title.as_deref(), headers, rows, area, buf),
        Node::Gauge {
            label,
            value,
            max,
            color,
        } => draw_gauge(label, *value, *max, *color, label_width, area, buf),
    }
}

fn row_areas(area: Rect, count: usize, gap: u16) -> Vec<Rect> {
    Layout::horizontal(vec![Constraint::Fill(1); count])
        .spacing(gap)
        .split(area)
        .to_vec()
}

fn paragraph<'a>(node: &Node, width: u16, cwd: &Path) -> Paragraph<'a> {
    let text = match node {
        Node::Text {
            text,
            bold,
            dim,
            italic,
            color,
        } => {
            let mut style = Style::default();
            if *bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            if *dim {
                style = style.add_modifier(Modifier::DIM);
            }
            if *italic {
                style = style.add_modifier(Modifier::ITALIC);
            }
            if let Some(color) = color {
                style = style.fg(to_color(*color));
            }
            Text::styled(text.clone(), style)
        }
        Node::Markdown { markdown } => {
            render_markdown_text_with_width_and_cwd(markdown, Some(usize::from(width)), Some(cwd))
        }
        _ => Text::default(),
    };
    Paragraph::new(text).wrap(Wrap { trim: false })
}

fn draw_table(
    title: Option<&str>,
    headers: &[String],
    rows: &[Vec<SpecCell>],
    area: Rect,
    buf: &mut Buffer,
) {
    let mut natural: Vec<u16> = headers.iter().map(|header| text_width(header)).collect();
    for row in rows {
        for (column, cell) in row.iter().enumerate() {
            if let Some(width) = natural.get_mut(column) {
                *width = (*width).max(text_width(&cell.0));
            }
        }
    }
    let spacing = TABLE_COLUMN_SPACING.saturating_mul(headers.len().saturating_sub(1) as u16);
    let content: u16 = natural.iter().copied().fold(0u16, u16::saturating_add);
    // Borders plus one column of padding on each side.
    let frame = 4;
    let available = area.width.saturating_sub(frame + spacing);
    // Columns keep their natural width when they fit, else share the width in proportion.
    let (widths, table_width): (Vec<Constraint>, u16) = if content <= available {
        (
            natural
                .iter()
                .map(|width| Constraint::Length(*width))
                .collect(),
            content + frame + spacing,
        )
    } else {
        (
            natural
                .iter()
                .map(|width| Constraint::Fill((*width).max(1)))
                .collect(),
            area.width,
        )
    };
    let header = Row::new(headers.iter().map(|header| Line::from(header.clone())))
        .style(Style::default().add_modifier(Modifier::BOLD));
    let body = rows.iter().map(|row| {
        Row::new(row.iter().map(|cell| {
            let text = cell.0.replace(['\n', '\r'], " ");
            if is_number(&text) {
                Line::from(text).right_aligned()
            } else {
                Line::from(text)
            }
        }))
    });
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().add_modifier(Modifier::DIM))
        .padding(Padding::horizontal(1));
    if let Some(title) = title.filter(|title| !title.trim().is_empty()) {
        block = block.title(Line::from(format!(" {title} ")).bold());
    }
    let table_area = Rect::new(
        area.x,
        area.y,
        table_width.min(area.width),
        (rows.len() as u16).saturating_add(3).min(area.height),
    );
    Table::new(body, widths)
        .header(header)
        .column_spacing(TABLE_COLUMN_SPACING)
        .block(block)
        .render(table_area, buf);
}

fn draw_gauge(
    label: &str,
    value: f64,
    max: f64,
    color: Option<SpecColor>,
    label_width: usize,
    area: Rect,
    buf: &mut Buffer,
) {
    let ratio = gauge_ratio(value, max);
    // A max of 100 or 1 reads as a percentage or a ratio; anything else as `value/max`.
    let value_text = if max == 100.0 || max == 1.0 {
        format!("{}%", (ratio * 100.0).round())
    } else {
        format!("{}/{}", number(value), number(max))
    };
    let label_width = label_width.max(UnicodeWidthStr::width(label));
    let value_width = GAUGE_VALUE_WIDTH.max(UnicodeWidthStr::width(value_text.as_str()));
    let padded_label = format!(
        "{label}{}",
        " ".repeat(label_width - UnicodeWidthStr::width(label))
    );
    let gauge_label = Line::from(vec![
        Span::styled(padded_label, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        Span::raw(format!("{value_text:>value_width$}")),
    ]);
    // LineGauge puts one space between the label and the bar.
    let label_columns = u16::try_from(label_width + 1 + value_width + 1).unwrap_or(u16::MAX);
    let width = area.width.min(label_columns.saturating_add(GAUGE_BAR_MAX));
    let fill = color.map(to_color).unwrap_or(Color::Green);
    LineGauge::default()
        .ratio(ratio)
        .label(gauge_label)
        .filled_symbol("█")
        .unfilled_symbol("░")
        .filled_style(Style::default().fg(fill))
        .unfilled_style(Style::default().add_modifier(Modifier::DIM))
        .render(Rect::new(area.x, area.y, width, 1), buf);
}

/// The widest label among `children` that are gauges, so their bars line up.
fn gauge_label_width(children: &[Node]) -> usize {
    children
        .iter()
        .filter_map(|child| match child {
            Node::Gauge { label, .. } => Some(UnicodeWidthStr::width(label.as_str())),
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

/// Converts buffer rows to lines, merging runs of one style and dropping trailing blanks.
fn buffer_lines(buf: &Buffer) -> Vec<Line<'static>> {
    let area = buf.area;
    let mut lines = Vec::with_capacity(usize::from(area.height));
    for y in area.top()..area.bottom() {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut skip = 0usize;
        for x in area.left()..area.right() {
            if skip > 0 {
                // The cells covered by a wide character.
                skip -= 1;
                continue;
            }
            let cell = &buf[(x, y)];
            let symbol = cell.symbol();
            skip = UnicodeWidthStr::width(symbol).saturating_sub(1);
            let mut style = Style::default().add_modifier(cell.modifier);
            if cell.fg != Color::Reset {
                style = style.fg(cell.fg);
            }
            if cell.bg != Color::Reset {
                style = style.bg(cell.bg);
            }
            match spans.last_mut() {
                Some(last) if last.style == style => last.content.to_mut().push_str(symbol),
                _ => spans.push(Span::styled(symbol.to_string(), style)),
            }
        }
        while let Some(last) = spans.last_mut() {
            if last.style != Style::default() {
                break;
            }
            let trimmed = last.content.trim_end().to_string();
            if trimmed.is_empty() {
                spans.pop();
            } else {
                last.content = trimmed.into();
                break;
            }
        }
        lines.push(Line::from(spans));
    }
    while lines.last().is_some_and(|line| line.spans.is_empty()) {
        lines.pop();
    }
    lines
}

fn to_color(color: SpecColor) -> Color {
    match color {
        SpecColor::Black => Color::Black,
        SpecColor::Red => Color::Red,
        SpecColor::Green => Color::Green,
        SpecColor::Yellow => Color::Yellow,
        SpecColor::Blue => Color::Blue,
        SpecColor::Magenta => Color::Magenta,
        SpecColor::Cyan => Color::Cyan,
        SpecColor::Gray => Color::Gray,
        SpecColor::White => Color::White,
    }
}

fn text_width(text: &str) -> u16 {
    u16::try_from(UnicodeWidthStr::width(text)).unwrap_or(u16::MAX)
}

fn is_number(text: &str) -> bool {
    let trimmed = text.trim().trim_end_matches('%');
    !trimmed.is_empty() && trimmed.parse::<f64>().is_ok()
}

fn number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        let text = format!("{value:.2}");
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// The view as plain text, for the non-interactive CLI.
pub fn render_ui_plain(spec: &serde_json::Value, width: u16) -> Result<Vec<String>, String> {
    let node = ether_ui_spec::parse(spec)?;
    // No terminal colour probe: plain output carries no colours.
    crate::terminal_palette::set_default_colors_from_startup_probe(None);
    let cwd = std::env::current_dir().unwrap_or_default();
    Ok(render_lines(&node, width, &cwd)
        .into_iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect())
}
