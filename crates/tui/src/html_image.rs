// New in ether: HTML views shown as images in the terminal.
//! A ```` ```view ```` block in an answer is rendered to a PNG (`view_renderer`) and shown inline
//! with kitty's graphics protocol, through Unicode placeholders (`U+10EEEE` cells whose
//! foreground colour carries the image id), so it is ordinary text to the TUI: it goes into the
//! scrollback like any other history line and scrolls with it. This module holds the terminal
//! side: capability detection, the model's instructions, sizing, and the kitty encoding.

use crate::kitty_diacritics::ROW_COLUMN_DIACRITICS;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use std::path::PathBuf;

const PLACEHOLDER: char = '\u{10EEEE}';
/// Tallest image, in terminal rows.
pub(crate) const MAX_ROWS: u16 = 120;

/// How views should look to match the terminal.
#[derive(Debug, Clone)]
pub(crate) struct PageStyle {
    /// Text and background colours.
    pub(crate) fg: (u8, u8, u8),
    pub(crate) bg: (u8, u8, u8),
    pub(crate) dark: bool,
}

impl PageStyle {
    /// From the terminal's default colours when known; otherwise a dark background is assumed.
    pub(crate) fn from_terminal(fg: Option<(u8, u8, u8)>, bg: Option<(u8, u8, u8)>) -> Self {
        let dark = bg.is_none_or(|(r, g, b)| is_dark(r, g, b));
        let fg = fg.unwrap_or(if dark { (0xe4, 0xe4, 0xe4) } else { (0x1f, 0x1f, 0x1f) });
        let bg = bg.unwrap_or(if dark { (0x1d, 0x1f, 0x21) } else { (0xff, 0xff, 0xff) });
        Self { fg, bg, dark }
    }
}

/// What the model is told when views are drawn (the `<terminal_display>` context section).
/// `dark` is the terminal background, if known. The text is fixed for a session: it is part of
/// the prompt-cache prefix.
pub fn view_instructions(dark: Option<bool>, width_px: Option<u32>) -> String {
    let background = match dark {
        Some(true) => "dark",
        Some(false) => "light",
        None => "terminal's",
    };
    let width = match width_px {
        Some(width) => format!("about {width}px"),
        None => "the terminal's width".to_string(),
    };
    format!(
        "Your answers are shown in a terminal that draws views: put HTML in a ```view block \
         (not ```html, which is shown as code) and the user sees it rendered as an image in its \
         place, styled to match the terminal ({background} background). Static only (no \
         JavaScript, no external URLs). Views are drawn {width} wide.\n\
         Diagrams (architecture, flows, sequences, states, timelines) go in a ```mermaid block \
         instead of HTML; they are laid out and drawn for you. Keep labels short.\n\
         In views, write semantic HTML with these building blocks; avoid inline colours, fixed \
         pixel sizes and hand-drawn SVG:\n\
         - layout: .row (side by side), .stack (vertical), .grid (CSS grid; set \
         grid-template-columns), .card (a box; tone-ok|warn|err|info|accent|violet tints it), \
         .badge with ok|warn|err|info, .muted\n\
         - <x-table> with a markdown table inside: a styled table, numbers right-aligned\n\
         - <x-stat label=\"…\" value=\"…\" tone=\"ok\" hint=\"…\">: a status tile\n\
         - <x-bars unit=\"…\"><x-bar label=\"…\" value=\"…\"></x-bar>…</x-bars>: a bar chart, \
         scaled for you\n\
         - <x-gauge label=\"…\" value=\"…\" max=\"100\" type=\"ring|bar\" tone=\"ok\">\n\
         - <x-spark values=\"3 5 2 8\">: a sparkline\n\
         - colours, if needed: var(--ok) var(--warn) var(--err) var(--info) var(--accent) \
         var(--violet) var(--muted)\n\
         When the answer has numbers to compare, show them in a view instead of listing them; \
         also use views for dashboards and side-by-side comparisons. Short answers, simple \
         lists, code and text the user may copy stay markdown. Do not repeat a view's numbers \
         in prose; add at most one sentence."
    )
}

/// Device pixels per CSS pixel, chosen so one cell is 17 CSS pixels tall: 13px CSS text then
/// matches the terminal's text whatever its font size or pixel density.
pub(crate) fn device_scale(cell_height: f64) -> f64 {
    (cell_height / 17.0).max(0.5)
}

/// The CSS width a view gets at `cols` terminal columns.
pub(crate) fn css_width_for(cols: u16, (cell_width, cell_height): (f64, f64)) -> u32 {
    ((f64::from(cols) * cell_width) / device_scale(cell_height)).floor() as u32
}

/// The width views are drawn at in the current terminal, in CSS pixels rounded to 50, for the
/// instructions (`None` if the terminal does not report its pixel size).
pub fn view_width_px() -> Option<u32> {
    let cell = cell_pixels()?;
    let (columns, _) = crossterm::terminal::size().ok()?;
    let width = css_width_for(columns.saturating_sub(2), cell);
    Some((width / 50).max(1) * 50)
}

/// Whether the terminal background is dark, from a colour query (`None` if it does not
/// answer). Call before the TUI starts: it briefly puts the terminal in raw mode. The answer
/// is cached for the page style.
pub fn terminal_is_dark() -> Option<bool> {
    crossterm::terminal::enable_raw_mode().ok()?;
    let background = crate::terminal_palette::default_bg();
    let _ = crossterm::terminal::disable_raw_mode();
    background.map(|(r, g, b)| is_dark(r, g, b))
}

fn is_dark(r: u8, g: u8, b: u8) -> bool {
    0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b) < 128.0
}

/// Whether this terminal can show views: image support, Chrome, and a known cell size.
/// Cheap, and safe before the TUI starts (it does not query the terminal).
pub fn views_available() -> bool {
    terminal_supports_images() && find_chrome().is_some() && cell_pixels().is_some()
}

/// A rendered view, ready to show: the PNG, its size in terminal cells, and what the view
/// kit's checks found.
#[derive(Debug)]
pub(crate) struct RenderedImage {
    pub(crate) png: Vec<u8>,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    pub(crate) issues: Vec<String>,
}

/// Whether the terminal shows images through Unicode placeholders (kitty, Ghostty).
/// `ETHER_IMAGES=0` turns images off, `ETHER_IMAGES=1` forces them on.
pub(crate) fn terminal_supports_images() -> bool {
    match std::env::var("ETHER_IMAGES").as_deref() {
        Ok("0") => return false,
        Ok("1") => return true,
        _ => {}
    }
    if std::env::var_os("TMUX").is_some() {
        return false;
    }
    let term = std::env::var("TERM").unwrap_or_default();
    term == "xterm-kitty"
        || term == "xterm-ghostty"
        || std::env::var_os("KITTY_WINDOW_ID").is_some()
        || std::env::var("TERM_PROGRAM").is_ok_and(|program| program == "ghostty")
}

/// The size of one terminal cell in device pixels, if the terminal reports it.
pub(crate) fn cell_pixels() -> Option<(f64, f64)> {
    let size = crossterm::terminal::window_size().ok()?;
    if size.width == 0 || size.height == 0 || size.columns == 0 || size.rows == 0 {
        return None;
    }
    Some((
        f64::from(size.width) / f64::from(size.columns),
        f64::from(size.height) / f64::from(size.rows),
    ))
}

/// A Chrome or Chromium binary: `ETHER_CHROME`, then the usual install locations.
pub(crate) fn find_chrome() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("ETHER_CHROME") {
        return Some(PathBuf::from(path));
    }
    let candidates = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ];
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

/// The escape sequence that uploads `png` as image `id` with a virtual placement of
/// `cols` x `rows` cells, for Unicode placeholders. `q=2` keeps the terminal quiet.
pub(crate) fn transmit(png: &[u8], id: u32, cols: u16, rows: u16) -> Vec<u8> {
    let data = STANDARD.encode(png);
    let chunks: Vec<&[u8]> = data.as_bytes().chunks(4096).collect();
    let mut out = Vec::with_capacity(data.len() + chunks.len() * 32);
    for (index, chunk) in chunks.iter().enumerate() {
        let more = u8::from(index + 1 < chunks.len());
        if index == 0 {
            out.extend_from_slice(
                format!("\x1b_Ga=T,U=1,f=100,t=d,q=2,i={id},c={cols},r={rows},m={more};")
                    .as_bytes(),
            );
        } else {
            out.extend_from_slice(format!("\x1b_Gm={more};").as_bytes());
        }
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
    out
}

/// The placeholder text for image `id`: one line per row, the first cell of each row carrying
/// the row diacritic; the terminal infers the columns.
// The foreground colour is the image id, not a display colour, so it must be exact RGB.
#[allow(clippy::disallowed_methods)]
pub(crate) fn placeholder_lines(id: u32, cols: u16, rows: u16) -> Vec<Line<'static>> {
    let color = Color::Rgb((id >> 16) as u8, (id >> 8) as u8, id as u8);
    (0..rows)
        .map(|row| {
            let mut text = String::new();
            text.push(PLACEHOLDER);
            if let Some(diacritic) = ROW_COLUMN_DIACRITICS.get(usize::from(row)) {
                text.push(*diacritic);
            }
            for _ in 1..cols {
                text.push(PLACEHOLDER);
            }
            Line::from(Span::styled(text, Style::default().fg(color)))
        })
        .collect()
}

/// A fresh image id that fits a 24-bit colour and is never 0.
pub(crate) fn new_image_id() -> u32 {
    rand::random::<u32>() % 0x00ff_fffe + 1
}

/// A rendered HTML view in history: placeholder text the terminal draws the image over.
#[derive(Debug)]
pub(crate) struct ImageCell {
    lines: Vec<Line<'static>>,
}

impl ImageCell {
    pub(crate) fn new(id: u32, image: &RenderedImage) -> Self {
        Self {
            lines: placeholder_lines(id, image.cols, image.rows),
        }
    }
}

impl crate::history_cell::HistoryCell for ImageCell {
    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        self.lines
            .iter()
            .map(|line| {
                let mut spans = vec![Span::raw("  ")];
                spans.extend(line.spans.iter().cloned());
                Line::from(spans)
            })
            .collect()
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        vec![Line::from("  [html view]")]
    }
}
