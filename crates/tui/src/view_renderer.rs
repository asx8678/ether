// New in ether: draws ```view and ```mermaid blocks with one warm headless Chrome.
//! A background thread owns the browser (started when the TUI starts, so the first view does
//! not wait for Chrome) and draws one block at a time:
//!
//! - a view: load the model's HTML in a fresh page, run the view kit runtime, screenshot;
//! - a diagram: hand the Mermaid source to a page where Mermaid is already loaded, screenshot.
//!
//! Screenshots are clipped to the drawn size and scaled by Chrome to whole terminal cells.
//!
//! Only static content from the model is shown: the pages' CSP (`default-src 'none'`, no script
//! source) blocks every script, inline handler and `javascript:` URL the model writes. The view
//! kit runtime and Mermaid are ether's own code, sent over the DevTools pipe; diagram source is
//! passed to Mermaid as a string (strict security level), never as code. Chrome runs offline:
//! a throwaway profile, a CSP that blocks all loads, every host name unresolvable, and all
//! traffic sent to a dead proxy (which also stops navigation, such as a
//! `<meta http-equiv="refresh">` to an IP address, that a CSP does not cover).

use crate::devtools::Browser;
use crate::html_fence::BlockKind;
use crate::html_image::MAX_ROWS;
use crate::html_image::PageStyle;
use crate::html_image::RenderedImage;
use crate::html_image::css_width_for;
use crate::html_image::device_scale;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::Value;
use serde_json::json;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;

const KIT_CSS: &str = include_str!("view_kit/kit.css");
const KIT_RUNTIME: &str = include_str!("view_kit/runtime.js");
/// Mermaid 11.17.2 (MIT, see `view_kit/vendor/mermaid.LICENSE` and docs/PROVENANCE.md).
const MERMAID: &str = include_str!("view_kit/vendor/mermaid.min.js");
const STEP_TIMEOUT: Duration = Duration::from_secs(10);
/// A proxy address nothing listens on: every request Chrome makes fails.
const DEAD_PROXY: &str = "127.0.0.1:9";

type Rgb = (u8, u8, u8);

/// Accent colours for dark and light terminals.
struct Tones {
    accent: Rgb,
    ok: Rgb,
    warn: Rgb,
    err: Rgb,
    info: Rgb,
    violet: Rgb,
}

const DARK_TONES: Tones = Tones {
    accent: (0x7a, 0xa2, 0xf7),
    ok: (0x9e, 0xce, 0x6a),
    warn: (0xe0, 0xaf, 0x68),
    err: (0xf7, 0x76, 0x8e),
    info: (0x7d, 0xcf, 0xff),
    violet: (0xbb, 0x9a, 0xf7),
};

const LIGHT_TONES: Tones = Tones {
    accent: (0x2e, 0x5b, 0xd8),
    ok: (0x2f, 0x7d, 0x32),
    warn: (0x9a, 0x67, 0x00),
    err: (0xc6, 0x28, 0x39),
    info: (0x0b, 0x72, 0x85),
    violet: (0x70, 0x48, 0xe8),
};

/// A drawn block, tagged with its place in the answer.
pub(crate) type ViewResult = (u64, Result<RenderedImage, String>);

struct Job {
    seq: u64,
    kind: BlockKind,
    body: String,
    max_cols: u16,
}

/// The renderer thread's handle. Dropping it stops the thread after the current block.
pub(crate) struct ViewRenderer {
    jobs: Option<mpsc::Sender<Job>>,
    thread: Option<JoinHandle<()>>,
}

impl ViewRenderer {
    /// Starts the renderer thread, which starts Chrome (and loads Mermaid) right away.
    pub(crate) fn start(
        chrome: PathBuf,
        cell: (f64, f64),
        style: PageStyle,
        results: UnboundedSender<ViewResult>,
    ) -> Self {
        let (jobs, queue) = mpsc::channel::<Job>();
        let thread = std::thread::Builder::new()
            .name("ether-views".to_string())
            .spawn(move || {
                let mut renderer = ViewPage::open(&chrome, &style).ok();
                if let Some(open) = renderer.as_mut() {
                    // Warm up the diagram page too; a failure here is retried on first use.
                    let _ = open.diagram_session();
                }
                for job in queue {
                    if renderer.is_none() {
                        renderer = ViewPage::open(&chrome, &style).ok();
                    }
                    let result = match renderer.as_mut() {
                        Some(open) => open.render(&job, cell),
                        None => Err("cannot start Chrome".to_string()),
                    };
                    if result.as_ref().is_err_and(|err| err.starts_with("browser:")) {
                        // The browser itself failed: start a fresh one for the next block.
                        renderer = None;
                    }
                    let _ = results.send((job.seq, result));
                }
            })
            .ok();
        Self {
            jobs: Some(jobs),
            thread,
        }
    }

    /// Queues a block; its result arrives on the results channel with `seq`.
    pub(crate) fn render(&self, seq: u64, kind: BlockKind, body: String, max_cols: u16) -> bool {
        self.jobs.as_ref().is_some_and(|jobs| {
            jobs.send(Job {
                seq,
                kind,
                body,
                max_cols,
            })
            .is_ok()
        })
    }

    /// Stops the thread and Chrome, waiting for the block being drawn, if any.
    pub(crate) fn shutdown(mut self) {
        self.jobs.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Chrome with a tab for views and, once needed, a tab with Mermaid loaded.
struct ViewPage {
    browser: Browser,
    view_session: String,
    diagram_session: Option<String>,
    style: PageStyle,
}

impl ViewPage {
    fn open(chrome: &Path, style: &PageStyle) -> Result<Self, String> {
        let args = [
            "--host-resolver-rules=MAP * ~NOTFOUND".to_string(),
            format!("--proxy-server={DEAD_PROXY}"),
            "--proxy-bypass-list=<-loopback>".to_string(),
            "--hide-scrollbars".to_string(),
            "--no-first-run".to_string(),
            "--no-default-browser-check".to_string(),
            "--disable-extensions".to_string(),
            "--disable-sync".to_string(),
            "--disable-background-networking".to_string(),
            "--disable-component-update".to_string(),
            "--mute-audio".to_string(),
        ];
        let mut browser = Browser::launch(chrome, &args)?;
        let view_session = new_tab(&mut browser)?;
        Ok(Self {
            browser,
            view_session,
            diagram_session: None,
            style: style.clone(),
        })
    }

    fn render(&mut self, job: &Job, cell: (f64, f64)) -> Result<RenderedImage, String> {
        let scale = device_scale(cell.1);
        let css_width = css_width_for(job.max_cols, cell);
        let (session, drawn) = match job.kind {
            BlockKind::View => {
                let session = self.view_session.clone();
                (session.clone(), self.draw_view(&session, &job.body, css_width, scale)?)
            }
            BlockKind::Mermaid => {
                let session = self.diagram_session()?;
                (session.clone(), self.draw_diagram(&session, &job.body, css_width, scale)?)
            }
        };
        self.capture(&session, &drawn, job.max_cols, cell, scale)
    }

    /// Loads the model's HTML in the view tab and runs the view kit over it.
    fn draw_view(
        &mut self,
        session: &str,
        html: &str,
        css_width: u32,
        scale: f64,
    ) -> Result<Value, String> {
        // Flowing content wraps at `css_width`; the window is wider so a view with fixed widths
        // is captured whole and scaled down instead of being clipped.
        self.set_metrics(session, css_width + css_width / 2, scale)?;
        let path = self.browser.profile().join("view.html");
        self.load(session, &path, &page_html(html, css_width, &self.style))?;
        let evaluated = self
            .browser
            .call(
                "Runtime.evaluate",
                json!({ "expression": KIT_RUNTIME, "returnByValue": true }),
                Some(session),
                STEP_TIMEOUT,
            )
            .map_err(browser_error)?;
        if let Some(exception) = evaluated.get("exceptionDetails") {
            return Err(format!("view kit: {}", exception["exception"]["description"]));
        }
        Ok(evaluated["result"]["value"].clone())
    }

    /// Draws Mermaid source in the diagram tab.
    fn draw_diagram(
        &mut self,
        session: &str,
        source: &str,
        css_width: u32,
        scale: f64,
    ) -> Result<Value, String> {
        self.set_metrics(session, css_width, scale)?;
        // The source goes in as a JSON string literal: data for Mermaid, never code.
        let source = serde_json::to_string(source).map_err(|err| err.to_string())?;
        let expression = format!(
            "(async (source, width) => {{\n\
               document.body.style.width = width + 'px';\n\
               const box = document.getElementById('diagram');\n\
               const id = 'ether-diagram-' + (window.etherDiagrams = (window.etherDiagrams || 0) + 1);\n\
               try {{\n\
                 const {{ svg }} = await mermaid.render(id, source);\n\
                 box.innerHTML = svg;\n\
               }} catch (error) {{\n\
                 box.innerHTML = '';\n\
                 for (const stray of document.querySelectorAll('[id^=\"dether-diagram\"]')) stray.remove();\n\
                 return {{ error: String((error && error.message) || error).split('\\n').slice(0, 4).join(' ') }};\n\
               }}\n\
               const r = box.firstElementChild.getBoundingClientRect();\n\
               return {{ width: Math.ceil(r.right) + 2, height: Math.ceil(r.bottom) + 2, issues: [] }};\n\
             }})({source}, {css_width})"
        );
        let evaluated = self
            .browser
            .call(
                "Runtime.evaluate",
                json!({ "expression": expression, "awaitPromise": true, "returnByValue": true }),
                Some(session),
                STEP_TIMEOUT,
            )
            .map_err(browser_error)?;
        let drawn = evaluated["result"]["value"].clone();
        if let Some(error) = drawn.get("error").and_then(Value::as_str) {
            return Err(format!("mermaid: {error}"));
        }
        Ok(drawn)
    }

    /// The tab with Mermaid loaded and themed, created on first use.
    fn diagram_session(&mut self) -> Result<String, String> {
        if let Some(session) = &self.diagram_session {
            return Ok(session.clone());
        }
        let session = new_tab(&mut self.browser)?;
        let path = self.browser.profile().join("diagram.html");
        self.load(
            &session,
            &path,
            &page_html("<div id=\"diagram\"></div>", 800, &self.style),
        )?;
        let config = mermaid_config(&self.style);
        let evaluated = self
            .browser
            .call(
                "Runtime.evaluate",
                json!({
                    "expression": format!("{MERMAID}\n;mermaid.initialize({config}); 'ready'"),
                    "returnByValue": true
                }),
                Some(&session),
                Duration::from_secs(30),
            )
            .map_err(browser_error)?;
        if evaluated["result"]["value"].as_str() != Some("ready") {
            return Err("mermaid did not load".to_string());
        }
        self.diagram_session = Some(session.clone());
        Ok(session)
    }

    fn set_metrics(&mut self, session: &str, width: u32, scale: f64) -> Result<(), String> {
        self.browser
            .call(
                "Emulation.setDeviceMetricsOverride",
                json!({ "width": width, "height": 800, "deviceScaleFactor": scale, "mobile": false }),
                Some(session),
                STEP_TIMEOUT,
            )
            .map(|_| ())
            .map_err(browser_error)
    }

    /// Writes `html` to `path` and loads it in the tab.
    fn load(&mut self, session: &str, path: &Path, html: &str) -> Result<(), String> {
        std::fs::write(path, html).map_err(|err| format!("write page: {err}"))?;
        let url = url::Url::from_file_path(path).map_err(|()| "bad page path".to_string())?;
        self.browser.clear_events();
        let navigated = self
            .browser
            .call("Page.navigate", json!({ "url": url.as_str() }), Some(session), STEP_TIMEOUT)
            .map_err(browser_error)?;
        if let Some(error) = navigated.get("errorText").and_then(Value::as_str) {
            return Err(format!("load page: {error}"));
        }
        self.browser
            .wait_event("Page.loadEventFired", session, STEP_TIMEOUT)
            .map(|_| ())
            .map_err(browser_error)
    }

    /// Screenshots the drawn area. Chrome scales and clips the capture itself: a block wider
    /// than the terminal is drawn smaller (text stays sharp), and the clip covers whole cells,
    /// so the PNG goes to the terminal as it is, already the size of its cell grid.
    fn capture(
        &mut self,
        session: &str,
        drawn: &Value,
        max_cols: u16,
        (cell_width, cell_height): (f64, f64),
        scale: f64,
    ) -> Result<RenderedImage, String> {
        let issues: Vec<String> = drawn["issues"]
            .as_array()
            .map(|issues| {
                issues
                    .iter()
                    .filter_map(|issue| issue.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let width = drawn["width"].as_f64().unwrap_or(0.0);
        let height = drawn["height"].as_f64().unwrap_or(0.0);
        if width < 1.0 || height < 1.0 {
            return Err("nothing was drawn".to_string());
        }
        let shrink = (f64::from(max_cols) * cell_width / (width * scale)).min(1.0);
        let cols = ((width * scale * shrink / cell_width).ceil() as u16).clamp(1, max_cols);
        let rows = ((height * scale * shrink / cell_height).ceil() as u16).clamp(1, MAX_ROWS);
        let clip_width = f64::from(cols) * cell_width / (scale * shrink);
        let clip_height = f64::from(rows) * cell_height / (scale * shrink);
        let shot = self
            .browser
            .call(
                "Page.captureScreenshot",
                json!({
                    "format": "png",
                    "captureBeyondViewport": true,
                    "clip": { "x": 0, "y": 0, "width": clip_width, "height": clip_height, "scale": shrink }
                }),
                Some(session),
                STEP_TIMEOUT,
            )
            .map_err(browser_error)?;
        let png = STANDARD
            .decode(shot["data"].as_str().unwrap_or_default())
            .map_err(|err| format!("screenshot: {err}"))?;
        Ok(RenderedImage {
            png,
            cols,
            rows,
            issues,
        })
    }
}

/// Marks errors from the browser connection, after which the browser is restarted.
fn browser_error(err: String) -> String {
    format!("browser: {err}")
}

/// Opens a tab with a transparent background and returns its session id.
fn new_tab(browser: &mut Browser) -> Result<String, String> {
    let target = browser
        .call("Target.createTarget", json!({ "url": "about:blank" }), None, STEP_TIMEOUT)
        .map_err(browser_error)?;
    let target_id = target["targetId"].as_str().ok_or("browser: no target id")?.to_string();
    let attached = browser
        .call(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
            None,
            STEP_TIMEOUT,
        )
        .map_err(browser_error)?;
    let session = attached["sessionId"].as_str().ok_or("browser: no session id")?.to_string();
    browser
        .call("Page.enable", json!({}), Some(&session), STEP_TIMEOUT)
        .map_err(browser_error)?;
    browser
        .call(
            "Emulation.setDefaultBackgroundColorOverride",
            json!({ "color": { "r": 0, "g": 0, "b": 0, "a": 0 } }),
            Some(&session),
            STEP_TIMEOUT,
        )
        .map_err(browser_error)?;
    Ok(session)
}

fn tones(style: &PageStyle) -> &'static Tones {
    if style.dark { &DARK_TONES } else { &LIGHT_TONES }
}

fn hex((r, g, b): Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// `amount` of colour `a` mixed into `b`, as hex.
fn mix(a: Rgb, b: Rgb, amount: f64) -> String {
    let channel = |x: u8, y: u8| (f64::from(x) * amount + f64::from(y) * (1.0 - amount)).round() as u8;
    hex((channel(a.0, b.0), channel(a.1, b.1), channel(a.2, b.2)))
}

/// The model's HTML in a page with the view kit's theme and no network access.
fn page_html(html: &str, css_width: u32, style: &PageStyle) -> String {
    let scheme = if style.dark { "dark" } else { "light" };
    let t = tones(style);
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\">\n\
         <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:\">\n\
         <style>:root{{--fg:{fg};--bg:{bg};color-scheme:{scheme};--accent:{accent};--ok:{ok};--warn:{warn};--err:{err};--info:{info};--violet:{violet}}}\n\
         {KIT_CSS}\nbody{{width:{css_width}px}}</style>\n</head><body>\n{html}\n</body></html>",
        fg = hex(style.fg),
        bg = hex(style.bg),
        accent = hex(t.accent),
        ok = hex(t.ok),
        warn = hex(t.warn),
        err = hex(t.err),
        info = hex(t.info),
        violet = hex(t.violet),
    )
}

/// Mermaid's configuration: strict security, and a theme from the terminal's colours.
fn mermaid_config(style: &PageStyle) -> Value {
    let (fg, bg) = (style.fg, style.bg);
    let t = tones(style);
    let surface = mix(fg, bg, 0.09);
    let line = mix(fg, bg, 0.28);
    let pairs: Vec<(&str, String)> = vec![
        ("background", hex(bg)),
        ("fontSize", "13px".to_string()),
        ("textColor", hex(fg)),
        ("primaryColor", surface.clone()),
        ("primaryTextColor", hex(fg)),
        ("primaryBorderColor", hex(t.accent)),
        ("secondaryColor", mix(t.accent, bg, 0.14)),
        ("secondaryTextColor", hex(fg)),
        ("secondaryBorderColor", mix(t.accent, bg, 0.6)),
        ("tertiaryColor", mix(fg, bg, 0.04)),
        ("tertiaryTextColor", hex(fg)),
        ("tertiaryBorderColor", line.clone()),
        ("mainBkg", surface.clone()),
        ("nodeBorder", hex(t.accent)),
        ("nodeTextColor", hex(fg)),
        ("lineColor", mix(fg, bg, 0.55)),
        ("clusterBkg", mix(fg, bg, 0.04)),
        ("clusterBorder", line.clone()),
        ("titleColor", hex(fg)),
        ("edgeLabelBackground", hex(bg)),
        ("actorBkg", surface.clone()),
        ("actorBorder", hex(t.accent)),
        ("actorTextColor", hex(fg)),
        ("actorLineColor", mix(t.accent, bg, 0.7)),
        ("signalColor", hex(fg)),
        ("signalTextColor", hex(fg)),
        ("labelBoxBkgColor", surface),
        ("labelBoxBorderColor", line.clone()),
        ("labelTextColor", hex(fg)),
        ("loopTextColor", hex(fg)),
        ("noteBkgColor", mix(t.warn, bg, 0.16)),
        ("noteTextColor", hex(fg)),
        ("noteBorderColor", mix(t.warn, bg, 0.55)),
        ("activationBkgColor", mix(t.accent, bg, 0.22)),
        ("activationBorderColor", hex(t.accent)),
        ("pie1", hex(t.accent)),
        ("pie2", hex(t.ok)),
        ("pie3", hex(t.warn)),
        ("pie4", hex(t.violet)),
        ("pie5", hex(t.info)),
        ("pie6", hex(t.err)),
        ("pieTitleTextColor", hex(fg)),
        ("pieLegendTextColor", hex(fg)),
        ("pieSectionTextColor", hex(bg)),
        ("pieStrokeColor", hex(bg)),
        ("taskBkgColor", mix(t.accent, bg, 0.35)),
        ("taskBorderColor", hex(t.accent)),
        ("taskTextColor", hex(fg)),
        ("taskTextOutsideColor", hex(fg)),
        ("sectionBkgColor", mix(fg, bg, 0.05)),
        ("altSectionBkgColor", hex(bg)),
        ("gridColor", line.clone()),
        ("doneTaskBkgColor", mix(t.ok, bg, 0.35)),
        ("doneTaskBorderColor", hex(t.ok)),
        ("critBkgColor", mix(t.err, bg, 0.35)),
        ("critBorderColor", hex(t.err)),
        ("todayLineColor", hex(t.err)),
    ];
    let mut variables: serde_json::Map<String, Value> = pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), Value::String(value)))
        .collect();
    variables.insert("darkMode".to_string(), Value::Bool(style.dark));
    let palette = [t.accent, t.ok, t.warn, t.violet, t.info, t.err].map(hex).join(", ");
    variables.insert(
        "xyChart".to_string(),
        json!({
            "backgroundColor": "transparent",
            "titleColor": hex(fg),
            "xAxisLabelColor": hex(fg),
            "xAxisTitleColor": hex(fg),
            "xAxisTickColor": line,
            "xAxisLineColor": line,
            "yAxisLabelColor": hex(fg),
            "yAxisTitleColor": hex(fg),
            "yAxisTickColor": line,
            "yAxisLineColor": line,
            "plotColorPalette": palette
        }),
    );
    json!({
        "startOnLoad": false,
        "securityLevel": "strict",
        "theme": "base",
        "fontFamily": "-apple-system, system-ui, \"Segoe UI\", sans-serif",
        "themeVariables": variables
    })
}
