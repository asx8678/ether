// New in ether: renders ```view blocks with one warm headless Chrome and the view kit.
//! A background thread owns the browser (started when the TUI starts, so the first view does
//! not wait for Chrome) and renders one view at a time: it loads the page, runs the view kit
//! runtime through the DevTools pipe, and takes a screenshot clipped to the drawn size.
//!
//! Only static HTML and CSS from the model is rendered: the page's CSP (`default-src 'none'`,
//! no script source) blocks every script, inline handler and `javascript:` URL the model
//! writes; the view kit runtime is ether's own code, sent over the pipe. Chrome runs offline:
//! a throwaway profile, a CSP that blocks all loads, every host name unresolvable, and all
//! traffic sent to a dead proxy (which also stops navigation, such as a
//! `<meta http-equiv="refresh">` to an IP address, that a CSP does not cover).

use crate::devtools::Browser;
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
const STEP_TIMEOUT: Duration = Duration::from_secs(10);
/// A proxy address nothing listens on: every request Chrome makes fails.
const DEAD_PROXY: &str = "127.0.0.1:9";
/// Tones for dark and light terminals.
const DARK_TONES: &str = "--accent:#7aa2f7;--ok:#9ece6a;--warn:#e0af68;--err:#f7768e;--info:#7dcfff;--violet:#bb9af7";
const LIGHT_TONES: &str = "--accent:#2e5bd8;--ok:#2f7d32;--warn:#9a6700;--err:#c62839;--info:#0b7285;--violet:#7048e8";

/// A rendered view, tagged with its place in the answer.
pub(crate) type ViewResult = (u64, Result<RenderedImage, String>);

struct Job {
    seq: u64,
    html: String,
    max_cols: u16,
}

/// The renderer thread's handle. Dropping it stops the thread after the current view.
pub(crate) struct ViewRenderer {
    jobs: Option<mpsc::Sender<Job>>,
    thread: Option<JoinHandle<()>>,
}

impl ViewRenderer {
    /// Starts the renderer thread, which starts Chrome right away.
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
                let mut page = ViewPage::open(&chrome).ok();
                for job in queue {
                    let result = match page.as_mut() {
                        Some(open) => open.render(&job.html, job.max_cols, cell, &style),
                        None => ViewPage::open(&chrome).and_then(|mut open| {
                            let result = open.render(&job.html, job.max_cols, cell, &style);
                            page = Some(open);
                            result
                        }),
                    };
                    if result.is_err() {
                        // Start a fresh browser for the next view.
                        page = None;
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

    /// Queues a view; its result arrives on the results channel with `seq`.
    pub(crate) fn render(&self, seq: u64, html: String, max_cols: u16) -> bool {
        self.jobs.as_ref().is_some_and(|jobs| {
            jobs.send(Job {
                seq,
                html,
                max_cols,
            })
            .is_ok()
        })
    }

    /// Stops the thread and Chrome, waiting for the view being rendered, if any.
    pub(crate) fn shutdown(mut self) {
        self.jobs.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A Chrome tab set up for views.
struct ViewPage {
    browser: Browser,
    session: String,
}

impl ViewPage {
    fn open(chrome: &Path) -> Result<Self, String> {
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
        let target = browser.call(
            "Target.createTarget",
            json!({ "url": "about:blank" }),
            None,
            STEP_TIMEOUT,
        )?;
        let target_id = target["targetId"].as_str().ok_or("no target id")?.to_string();
        let attached = browser.call(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
            None,
            STEP_TIMEOUT,
        )?;
        let session = attached["sessionId"].as_str().ok_or("no session id")?.to_string();
        browser.call("Page.enable", json!({}), Some(&session), STEP_TIMEOUT)?;
        browser.call(
            "Emulation.setDefaultBackgroundColorOverride",
            json!({ "color": { "r": 0, "g": 0, "b": 0, "a": 0 } }),
            Some(&session),
            STEP_TIMEOUT,
        )?;
        Ok(Self { browser, session })
    }

    fn render(
        &mut self,
        html: &str,
        max_cols: u16,
        cell: (f64, f64),
        style: &PageStyle,
    ) -> Result<RenderedImage, String> {
        let session = self.session.clone();
        let session = Some(session.as_str());
        let scale = device_scale(cell.1);
        let css_width = css_width_for(max_cols, cell);
        // Flowing content wraps at `css_width`; the window is wider so a view with fixed widths
        // is captured whole and scaled down instead of being clipped.
        let layout_width = css_width + css_width / 2;
        self.browser.call(
            "Emulation.setDeviceMetricsOverride",
            json!({ "width": layout_width, "height": 800, "deviceScaleFactor": scale, "mobile": false }),
            session,
            STEP_TIMEOUT,
        )?;
        let path = self.browser.profile().join("view.html");
        std::fs::write(&path, page_html(html, css_width, style))
            .map_err(|err| format!("write view: {err}"))?;
        let url = url::Url::from_file_path(&path).map_err(|()| "bad view path".to_string())?;
        self.browser.clear_events();
        let navigated = self.browser.call(
            "Page.navigate",
            json!({ "url": url.as_str() }),
            session,
            STEP_TIMEOUT,
        )?;
        if let Some(error) = navigated.get("errorText").and_then(Value::as_str) {
            return Err(format!("load view: {error}"));
        }
        self.browser
            .wait_event("Page.loadEventFired", &self.session, STEP_TIMEOUT)?;
        let evaluated = self.browser.call(
            "Runtime.evaluate",
            json!({ "expression": KIT_RUNTIME, "returnByValue": true }),
            session,
            STEP_TIMEOUT,
        )?;
        if let Some(exception) = evaluated.get("exceptionDetails") {
            return Err(format!("view kit: {}", exception["exception"]["description"]));
        }
        let drawn = &evaluated["result"]["value"];
        let issues: Vec<String> = drawn["issues"]
            .as_array()
            .map(|issues| issues.iter().filter_map(|issue| issue.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let (width, height) = (
            drawn["width"].as_f64().unwrap_or(0.0),
            drawn["height"].as_f64().unwrap_or(0.0),
        );
        if width < 1.0 || height < 1.0 {
            return Err("the view drew nothing".to_string());
        }
        // Chrome scales and clips the capture itself: a view wider than the terminal is drawn
        // smaller (text stays sharp), and the clip covers whole cells, so the PNG goes to the
        // terminal as it is, already the size of its cell grid.
        let (cell_width, cell_height) = cell;
        let shrink = (f64::from(max_cols) * cell_width / (width * scale)).min(1.0);
        let cols = ((width * scale * shrink / cell_width).ceil() as u16).clamp(1, max_cols);
        let rows = ((height * scale * shrink / cell_height).ceil() as u16).clamp(1, MAX_ROWS);
        let clip_width = f64::from(cols) * cell_width / (scale * shrink);
        let clip_height = f64::from(rows) * cell_height / (scale * shrink);
        let shot = self.browser.call(
            "Page.captureScreenshot",
            json!({
                "format": "png",
                "captureBeyondViewport": true,
                "clip": { "x": 0, "y": 0, "width": clip_width, "height": clip_height, "scale": shrink }
            }),
            session,
            STEP_TIMEOUT,
        )?;
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

/// The model's HTML in a page with the view kit's theme and no network access.
fn page_html(html: &str, css_width: u32, style: &PageStyle) -> String {
    let scheme = if style.dark { "dark" } else { "light" };
    let tones = if style.dark { DARK_TONES } else { LIGHT_TONES };
    format!(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\">\n\
         <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; img-src data:; font-src data:\">\n\
         <style>:root{{--fg:{fg};--bg:{bg};color-scheme:{scheme};{tones}}}\n{KIT_CSS}\nbody{{width:{css_width}px}}</style>\n\
         </head><body>\n{html}\n</body></html>",
        fg = style.fg,
        bg = style.bg,
    )
}
