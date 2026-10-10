// New in ether: HTML views rendered to images.
//! A ```` ```view ```` block of HTML in an answer is rendered by headless Chrome into a PNG and shown
//! inline with kitty's graphics protocol. The image is shown through Unicode placeholders
//! (`U+10EEEE` cells whose foreground colour carries the image id), so it is ordinary text to
//! the TUI: it goes into the scrollback like any other history line and scrolls with it.
//!
//! Only static HTML, CSS and inline SVG are rendered: the page's CSP (`default-src 'none'`,
//! no script source) blocks every script, inline handler and `javascript:` URL. (Chrome's
//! `scriptEnabled=false` setting also stops headless screenshots, so the CSP is the switch.)
//! Chrome runs offline: a throwaway profile, a CSP that blocks all loads, every host name
//! unresolvable, and all traffic sent to a dead proxy (which also stops navigation, such as a
//! `<meta http-equiv="refresh">` to an IP address, that a CSP does not cover), so model-written
//! HTML cannot reach anything outside the page.

use crate::kitty_diacritics::ROW_COLUMN_DIACRITICS;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

const PLACEHOLDER: char = '\u{10EEEE}';
/// Tallest image, in terminal rows.
const MAX_ROWS: u16 = 120;
/// Height of the page Chrome lays out before cropping, in CSS pixels.
const PAGE_HEIGHT: u32 = 2400;
const CHROME_TIMEOUT: Duration = Duration::from_secs(15);
/// A proxy address nothing listens on: every request Chrome makes fails.
const DEAD_PROXY: &str = "127.0.0.1:9";

/// How the page should look to match the terminal.
#[derive(Debug, Clone)]
pub(crate) struct PageStyle {
    /// CSS text colour.
    pub(crate) color: String,
    pub(crate) dark: bool,
}

impl PageStyle {
    /// From the terminal's default colours when known; otherwise a dark background is assumed.
    pub(crate) fn from_terminal(fg: Option<(u8, u8, u8)>, bg: Option<(u8, u8, u8)>) -> Self {
        let dark = bg.is_none_or(|(r, g, b)| is_dark(r, g, b));
        let color = match fg {
            Some((r, g, b)) => format!("rgb({r},{g},{b})"),
            None if dark => "#e4e4e4".to_string(),
            None => "#1f1f1f".to_string(),
        };
        Self { color, dark }
    }
}

/// What the model is told when views are drawn (the `<terminal_display>` context section).
/// `dark` is the terminal background, if known. The text is fixed for a session: it is part of
/// the prompt-cache prefix.
pub fn view_instructions(dark: Option<bool>) -> String {
    let background = match dark {
        Some(true) => "dark",
        Some(false) => "light",
        None => "terminal's",
    };
    format!(
        "Your answers are shown in a terminal that draws views: put HTML with inline CSS in a \
         ```view block and the user sees it rendered as an image in its place. Static only (no \
         JavaScript, no external URLs); inline SVG works for charts. The page is transparent over \
         the {background} background and text defaults to the terminal's colour.\n\
         When the answer has numbers to compare (sizes, percentages, counts, timings), show them \
         in a view with bars or gauges instead of listing them. Also use views for charts, \
         dashboards and side-by-side comparisons. Short answers, simple lists, code and text the \
         user may copy stay markdown; ```html blocks stay code. Do not repeat a view's numbers in \
         prose; add at most one sentence.\n\
         Example:\n\
         ```view\n\
         <div style=\"display:grid;grid-template-columns:auto 160px auto;gap:4px 10px;align-items:center\">\n\
         <span>disk</span><div style=\"background:#8884;height:10px\"><div style=\"width:63%;height:100%;background:#4c9\"></div></div><span>63%</span>\n\
         </div>\n\
         ```"
    )
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

/// A rendered view, ready to show: the PNG and its size in terminal cells.
#[derive(Debug)]
pub(crate) struct RenderedImage {
    pub(crate) png: Vec<u8>,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
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

/// Renders `html` at most `max_cols` cells wide. Blocking: runs Chrome.
pub(crate) fn render_html(
    chrome: &PathBuf,
    html: &str,
    max_cols: u16,
    cell: (f64, f64),
    style: &PageStyle,
) -> Result<RenderedImage, String> {
    let (cell_width, cell_height) = cell;
    // Device pixels per CSS pixel: about 2 on a Retina display, where cells are ~32px tall.
    let scale = (cell_height / 17.0).round().max(1.0);
    let css_width = ((f64::from(max_cols) * cell_width) / scale).floor() as u32;
    // Text about the size of the terminal's.
    let font_px = (cell_height / scale * 0.78).round().clamp(11.0, 24.0);
    let page = wrap_page(html, css_width, font_px, style);

    let dir = std::env::temp_dir().join(format!(
        "ether-html-{}-{}",
        std::process::id(),
        rand::random::<u32>()
    ));
    std::fs::create_dir_all(&dir).map_err(|err| format!("temp dir: {err}"))?;
    let result = render_in(chrome, &page, css_width, scale, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    let png = result?;

    let image = image::load_from_memory(&png).map_err(|err| format!("decode: {err}"))?;
    let (width, height) = drawn_extent(&image).ok_or("the page drew nothing")?;
    let cols = ((f64::from(width) / cell_width).ceil() as u16).clamp(1, max_cols);
    let rows = ((f64::from(height) / cell_height).ceil() as u16).clamp(1, MAX_ROWS);
    // Crop to the cell grid so kitty does not stretch the image.
    let crop_width = ((f64::from(cols) * cell_width).round() as u32).min(image.width());
    let crop_height = ((f64::from(rows) * cell_height).round() as u32).min(image.height());
    let cropped = image.crop_imm(0, 0, crop_width, crop_height);
    let mut out = Vec::new();
    cropped
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|err| format!("encode: {err}"))?;
    Ok(RenderedImage {
        png: out,
        cols,
        rows,
    })
}

fn render_in(
    chrome: &PathBuf,
    page_html: &str,
    css_width: u32,
    scale: f64,
    dir: &std::path::Path,
) -> Result<Vec<u8>, String> {
    let page = dir.join("view.html");
    let shot = dir.join("view.png");
    std::fs::write(&page, page_html).map_err(|err| format!("write: {err}"))?;
    let mut child = Command::new(chrome)
        .arg("--headless")
        .arg(format!("--screenshot={}", shot.display()))
        .arg(format!("--window-size={css_width},{PAGE_HEIGHT}"))
        .arg(format!("--force-device-scale-factor={scale}"))
        .arg("--default-background-color=00000000")
        .arg("--hide-scrollbars")
        .arg(format!("--user-data-dir={}", dir.join("profile").display()))
        .arg("--host-resolver-rules=MAP * ~NOTFOUND")
        .arg(format!("--proxy-server={DEAD_PROXY}"))
        .arg("--proxy-bypass-list=<-loopback>")
        .args([
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-extensions",
            "--disable-sync",
            "--disable-background-networking",
            "--disable-component-update",
            "--mute-audio",
        ])
        .arg(format!("file://{}", page.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("cannot start Chrome: {err}"))?;
    // Headless Chrome may keep running after writing the screenshot: wait until the file stops
    // growing, then stop Chrome.
    let deadline = Instant::now() + CHROME_TIMEOUT;
    let mut last_size = 0;
    let result = loop {
        let exited = !matches!(child.try_wait(), Ok(None));
        let size = std::fs::metadata(&shot).map(|meta| meta.len()).unwrap_or(0);
        if size > 0 && (exited || size == last_size) {
            // Accept the file once it is a complete PNG.
            if let Ok(bytes) = std::fs::read(&shot)
                && image::load_from_memory(&bytes).is_ok()
            {
                break Ok(bytes);
            }
        }
        if exited {
            break Err("Chrome exited without a screenshot".to_string());
        }
        if Instant::now() >= deadline {
            break Err("Chrome timed out".to_string());
        }
        last_size = size;
        std::thread::sleep(Duration::from_millis(50));
    };
    let _ = child.kill();
    let _ = child.wait();
    result
}

/// The model's HTML inside a page with terminal-friendly defaults and no network access.
fn wrap_page(html: &str, css_width: u32, font_px: f64, style: &PageStyle) -> String {
    let scheme = if style.dark { "dark" } else { "light" };
    let color = &style.color;
    format!(
        r#"<!doctype html>
<html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:">
<style>
html, body {{ margin: 0; background: transparent; color-scheme: {scheme}; }}
body {{ color: {color}; font: {font_px}px/1.45 -apple-system, system-ui, "Segoe UI", sans-serif;
  width: max-content; max-width: {css_width}px; padding: 2px 0; }}
table {{ border-collapse: collapse; }}
/* Plain tables get borders; a view with its own <style> keeps its own design. */
body:not(:has(style)) :where(th, td) {{ border: 1px solid rgba(160,160,160,.35); padding: 4px 10px; text-align: left; }}
body:not(:has(style)) :where(th) {{ background: rgba(160,160,160,.15); }}
code, pre {{ font-family: ui-monospace, Menlo, monospace; }}
</style></head><body>
{html}
</body></html>"#
    )
}

/// Right and bottom edge of the non-transparent pixels.
fn drawn_extent(image: &image::DynamicImage) -> Option<(u32, u32)> {
    let rgba = image.to_rgba8();
    let mut right = 0;
    let mut bottom = 0;
    let mut any = false;
    for (x, y, pixel) in rgba.enumerate_pixels() {
        if pixel[3] > 0 {
            any = true;
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
        }
    }
    any.then_some((right, bottom))
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
