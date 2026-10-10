// New in ether: the TUI app shell, chat state and composer, driving the engine through `Op` in and `EventMsg` out.
//! Finished history goes into the terminal scrollback above an inline viewport. The viewport
//! shows what is still live (the streaming answer's tail, running commands), a status line, and
//! either the composer or an approval prompt.

use crate::bottom_pane::textarea::TextArea;
use crate::bottom_pane::textarea::TextAreaState;
use crate::diff_model;
use crate::exec_cell::CommandOutput;
use crate::exec_cell::ExecCell;
use crate::exec_cell::new_active_exec_command;
use crate::history_cell;
use crate::html_fence::FenceOutput;
use crate::html_fence::HtmlFenceFilter;
use crate::html_image;
use crate::html_image::ImageCell;
use crate::html_image::PageStyle;
use crate::html_image::RenderedImage;
use crate::history_cell::HistoryCell;
use crate::history_cell::HistoryRenderMode;
use crate::streaming::controller::StreamController;
use crate::terminal_hyperlinks::HyperlinkLine;
use crate::terminal_hyperlinks::visible_lines;
use crate::tui::Tui;
use crate::ui_spec_cell::UiSpecCell;
use codex_protocol::items::AgentMessageContent;
use codex_protocol::items::TurnItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExecCommandBeginEvent;
use codex_protocol::protocol::ExecCommandEndEvent;
use codex_protocol::protocol::FileChange;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::ReviewDecision;
use codex_protocol::protocol::ThreadSettingsOverrides;
use codex_protocol::protocol::TokenUsageInfo;
use codex_protocol::protocol::UiRenderEvent;
use codex_protocol::turn_input::TurnInput;
use codex_protocol::turn_input::TurnInputMode;
use codex_protocol::turn_input::TurnInputRequest;
use codex_protocol::turn_input::TurnStartOptions;
use codex_protocol::user_input::UserInput;
use crossterm::event::Event;
use crossterm::event::EventStream;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ether_engine::Engine;
use futures::StreamExt;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::widgets::StatefulWidgetRef;
use ratatui::widgets::Widget;
use ratatui::widgets::Wrap;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

/// Width reserved by cells for their `• ` / `  ` gutter.
const STREAM_RESERVED_COLS: usize = 2;
/// Composer prompt gutter, and its width in columns.
const COMPOSER_PREFIX: &str = "› ";
const COMPOSER_PREFIX_WIDTH: u16 = 2;
const TICK: Duration = Duration::from_millis(50);

/// Runs the TUI until the user quits. `initial_prompt` is submitted right away.
pub async fn run(mut engine: Engine, initial_prompt: Option<String>) -> anyhow::Result<()> {
    let mut tui = Tui::init()?;
    let mut app = App::new();
    if let Some(prompt) = initial_prompt.filter(|prompt| !prompt.trim().is_empty()) {
        app.submit(&engine, prompt)?;
    }
    let result = app.run(&mut engine, &mut tui).await;
    let _ = tui.finish();
    drop(tui);
    // Let the engine write its rollout before the process exits.
    if engine.submit(Op::Shutdown).is_ok() {
        while let Some(event) = engine.next_event().await {
            if matches!(event.msg, EventMsg::ShutdownComplete) {
                break;
            }
        }
    }
    result
}

/// A question waiting on the user.
struct PendingApproval {
    kind: ApprovalKind,
    id: String,
    turn_id: String,
    question: String,
    reason: Option<String>,
}

enum ApprovalKind {
    Exec,
    Patch,
}

/// What HTML views need: a terminal that shows images, Chrome, and the cell size in pixels.
struct ImageSupport {
    chrome: PathBuf,
    cell: (f64, f64),
    style: PageStyle,
}

impl ImageSupport {
    /// Must run before the terminal event stream starts: it may query the terminal's colours.
    fn detect() -> Option<Self> {
        if !html_image::terminal_supports_images() {
            return None;
        }
        let chrome = html_image::find_chrome()?;
        let cell = html_image::cell_pixels()?;
        let style = PageStyle::from_terminal(
            crate::terminal_palette::default_fg(),
            crate::terminal_palette::default_bg(),
        );
        Some(Self {
            chrome,
            cell,
            style,
        })
    }
}

/// A rendered view, tagged with its place in the answer.
type ImageResult = (u64, Result<RenderedImage, String>);

/// Answer output waiting for an earlier view to finish rendering, kept in order.
enum Pending {
    Text(String),
    View {
        seq: u64,
        source: String,
        result: Option<Result<RenderedImage, String>>,
    },
}

struct App {
    cwd: PathBuf,
    model: String,
    provider: String,
    session_id: String,
    composer: TextArea,
    composer_state: TextAreaState,
    stream: Option<StreamController>,
    reasoning: BTreeMap<String, String>,
    exec_cells: HashMap<String, ExecCell>,
    /// `render_ui` views, live until later history is written after them or the turn ends.
    ui_views: Vec<UiSpecCell>,
    approval: Option<PendingApproval>,
    token_info: Option<TokenUsageInfo>,
    turn_started: Option<Instant>,
    /// History to write above the viewport on the next draw.
    pending_history: Vec<Box<dyn HistoryCell>>,
    /// Escape sequences (image uploads) to write before the next history insertion.
    pending_raw: Vec<Vec<u8>>,
    images: Option<ImageSupport>,
    /// Splits HTML blocks out of the streamed answer (only with image support).
    fence: HtmlFenceFilter,
    /// Views being rendered and the answer text after them, in answer order.
    pending: VecDeque<Pending>,
    next_view: u64,
    /// The current answer is still arriving.
    answer_open: bool,
    /// Some of the current answer arrived as deltas.
    answer_streamed: bool,
    image_tx: mpsc::UnboundedSender<ImageResult>,
    image_rx: Option<mpsc::UnboundedReceiver<ImageResult>>,
    width: u16,
    quit: bool,
}

impl App {
    fn new() -> Self {
        let (image_tx, image_rx) = mpsc::unbounded_channel();
        Self {
            cwd: std::env::current_dir().unwrap_or_default(),
            model: String::new(),
            provider: String::new(),
            session_id: String::new(),
            composer: TextArea::new(),
            composer_state: TextAreaState::default(),
            stream: None,
            reasoning: BTreeMap::new(),
            exec_cells: HashMap::new(),
            ui_views: Vec::new(),
            approval: None,
            token_info: None,
            turn_started: None,
            pending_history: Vec::new(),
            pending_raw: Vec::new(),
            images: ImageSupport::detect(),
            fence: HtmlFenceFilter::default(),
            pending: VecDeque::new(),
            next_view: 0,
            answer_open: false,
            answer_streamed: false,
            image_tx,
            image_rx: Some(image_rx),
            width: 80,
            quit: false,
        }
    }

    async fn run(&mut self, engine: &mut Engine, tui: &mut Tui) -> anyhow::Result<()> {
        let mut events = EventStream::new();
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let Some(mut image_rx) = self.image_rx.take() else {
            anyhow::bail!("the app already ran");
        };
        while !self.quit {
            self.draw(tui)?;
            tokio::select! {
                event = events.next() => match event {
                    Some(Ok(event)) => self.on_terminal_event(engine, event)?,
                    Some(Err(err)) => return Err(err.into()),
                    None => break,
                },
                event = engine.next_event() => match event {
                    Some(event) => self.on_engine_event(event.msg),
                    None => break,
                },
                Some(image) = image_rx.recv() => self.on_image(image),
                _ = tick.tick(), if self.is_animating() => self.on_tick(),
            }
        }
        self.flush_live_cells();
        self.draw(tui)?;
        Ok(())
    }

    fn is_animating(&self) -> bool {
        self.turn_started.is_some() || self.stream.is_some()
    }

    fn stream_width(&self) -> Option<usize> {
        Some(usize::from(self.width).saturating_sub(STREAM_RESERVED_COLS))
    }

    // ---- input ----------------------------------------------------------------------------

    fn on_terminal_event(&mut self, engine: &Engine, event: Event) -> anyhow::Result<()> {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => self.on_key(engine, key)?,
            Event::Paste(text) => {
                let text = text.replace("\r\n", "\n").replace('\r', "\n");
                let cursor = self.composer.cursor();
                self.composer.insert_str_at(cursor, &text);
            }
            Event::Resize(width, _) => {
                self.width = width;
                let stream_width = self.stream_width();
                if let Some(stream) = self.stream.as_mut() {
                    stream.set_width(stream_width);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn on_key(&mut self, engine: &Engine, key: KeyEvent) -> anyhow::Result<()> {
        if self.approval.is_some() {
            self.on_approval_key(engine, key);
            return Ok(());
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => {
                if self.turn_started.is_some() {
                    engine.submit(Op::Interrupt)?;
                } else if self.composer.is_empty() {
                    self.quit = true;
                } else {
                    self.composer.set_text_clearing_elements("");
                }
            }
            KeyCode::Char('d') if ctrl && self.composer.is_empty() => self.quit = true,
            KeyCode::Esc if self.turn_started.is_some() => engine.submit(Op::Interrupt)?,
            KeyCode::Enter
                if !key
                    .modifiers
                    .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
            {
                let text = self.composer.text().trim().to_string();
                if !text.is_empty() {
                    self.composer.set_text_clearing_elements("");
                    self.submit(engine, text)?;
                }
            }
            KeyCode::Enter => {
                let cursor = self.composer.cursor();
                self.composer.insert_str_at(cursor, "\n");
            }
            _ => self.composer.input(key),
        }
        Ok(())
    }

    fn on_approval_key(&mut self, engine: &Engine, key: KeyEvent) {
        let decision = match key.code {
            KeyCode::Char('y') | KeyCode::Enter => ReviewDecision::Approved,
            KeyCode::Char('a') => ReviewDecision::ApprovedForSession,
            KeyCode::Char('n') | KeyCode::Esc => ReviewDecision::Denied {
                rejection: "declined by the user".to_string(),
            },
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                ReviewDecision::Abort
            }
            _ => return,
        };
        let Some(approval) = self.approval.take() else {
            return;
        };
        let answer = match decision {
            ReviewDecision::Approved => "approved",
            ReviewDecision::ApprovedForSession => "approved for this session",
            ReviewDecision::Abort => "aborted",
            _ => "declined",
        };
        self.push_cell(history_cell::new_info_event(
            format!("{} — {answer}", approval.question),
            None,
        ));
        let op = match approval.kind {
            ApprovalKind::Exec => Op::ExecApproval {
                id: approval.id,
                turn_id: Some(approval.turn_id),
                decision,
            },
            ApprovalKind::Patch => Op::PatchApproval {
                id: approval.id,
                decision,
            },
        };
        let _ = engine.submit(op);
    }

    fn submit(&mut self, engine: &Engine, text: String) -> anyhow::Result<()> {
        let (reply, _reply_rx) = oneshot::channel();
        engine.submit(Op::TurnInput {
            request: Box::new(TurnInputRequest {
                input: TurnInput::UserInput {
                    content: vec![UserInput::Text {
                        text,
                        text_elements: Vec::new(),
                    }],
                    client_id: None,
                },
                thread_settings: ThreadSettingsOverrides::default(),
                start: TurnStartOptions::default(),
                additional_context: BTreeMap::new(),
                responsesapi_client_metadata: None,
                trace: None,
            }),
            mode: TurnInputMode::StartOrSteer,
            reply,
        })?;
        Ok(())
    }

    // ---- engine events --------------------------------------------------------------------

    fn on_engine_event(&mut self, msg: EventMsg) {
        match msg {
            EventMsg::SessionConfigured(configured) => {
                self.cwd = configured.cwd.to_path_buf();
                self.model = configured.model.clone();
                self.provider = configured.model_provider_id.clone();
                self.session_id = configured.session_id.to_string();
                self.push_cell(history_cell::new_info_event(
                    format!(
                        "ether · {} via {}",
                        configured.model, configured.model_provider_id
                    ),
                    Some(configured.cwd.display().to_string()),
                ));
            }
            EventMsg::TurnStarted(_) => self.turn_started = Some(Instant::now()),
            EventMsg::ItemCompleted(completed) => self.on_item_completed(completed.item),
            EventMsg::AgentMessageContentDelta(delta) => {
                self.answer_streamed = true;
                self.on_answer_text(&delta.delta);
            }
            EventMsg::ReasoningContentDelta(delta) => self
                .reasoning
                .entry(delta.item_id)
                .or_default()
                .push_str(&delta.delta),
            EventMsg::ReasoningRawContentDelta(delta) => self
                .reasoning
                .entry(delta.item_id)
                .or_default()
                .push_str(&delta.delta),
            EventMsg::ExecCommandBegin(begin) => self.on_exec_begin(begin),
            EventMsg::ExecCommandOutputDelta(delta) => {
                if let Some(cell) = self.exec_cells.get_mut(&delta.call_id) {
                    cell.append_output(&delta.call_id, &String::from_utf8_lossy(&delta.chunk));
                }
            }
            EventMsg::ExecCommandEnd(end) => self.on_exec_end(end),
            EventMsg::PatchApplyBegin(begin) => {
                let cwd = self.cwd.clone();
                self.push_cell(history_cell::new_patch_event(
                    convert_changes(begin.changes),
                    &cwd,
                ));
            }
            EventMsg::PatchApplyEnd(end) if !end.success => {
                let detail = if end.stderr.is_empty() {
                    end.stdout
                } else {
                    end.stderr
                };
                self.push_cell(history_cell::new_patch_apply_failure(detail));
            }
            EventMsg::PlanUpdate(update) => self.push_cell(history_cell::new_plan_update(update)),
            EventMsg::UiRender(event) => self.on_ui_render(event),
            EventMsg::ExecApprovalRequest(request) => {
                self.approval = Some(PendingApproval {
                    kind: ApprovalKind::Exec,
                    id: request.effective_approval_id(),
                    turn_id: request.turn_id.clone(),
                    question: format!("Run `{}`?", shlex_join(&request.command)),
                    reason: request.reason,
                });
            }
            EventMsg::ApplyPatchApprovalRequest(request) => {
                self.approval = Some(PendingApproval {
                    kind: ApprovalKind::Patch,
                    id: request.call_id.clone(),
                    turn_id: request.turn_id.clone(),
                    question: format!("Apply patch to {}?", describe_changes(&request.changes)),
                    reason: request.reason,
                });
            }
            EventMsg::TokenCount(count) if count.info.is_some() => self.token_info = count.info,
            EventMsg::Warning(warning) => {
                self.push_cell(history_cell::new_warning_event(warning.message))
            }
            EventMsg::StreamError(error) => self.push_cell(history_cell::new_info_event(
                error.message,
                error.additional_details,
            )),
            EventMsg::Error(error) => self.push_cell(history_cell::new_error_event(error.message)),
            EventMsg::TurnComplete(_) => self.end_turn(None),
            EventMsg::TurnAborted(_) => self.end_turn(Some("interrupted")),
            EventMsg::ShutdownComplete => self.quit = true,
            _ => {}
        }
    }

    fn on_item_completed(&mut self, item: TurnItem) {
        match item {
            TurnItem::UserMessage(message) => {
                self.push_cell(history_cell::new_user_prompt(
                    message.message(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                ));
            }
            TurnItem::AgentMessage(message) => {
                let text: String = message
                    .content
                    .iter()
                    .map(|part| match part {
                        AgentMessageContent::Text { text } => text.as_str(),
                    })
                    .collect();
                if !self.answer_streamed {
                    // Nothing was streamed for this message; render it whole.
                    self.on_answer_text(&text);
                }
                for output in self.fence.finish() {
                    self.on_fence_output(output);
                }
                self.answer_open = false;
                self.answer_streamed = false;
                if self.pending.is_empty() {
                    self.finish_stream();
                }
            }
            TurnItem::Reasoning(reasoning) => {
                let mut text = self.reasoning.remove(&reasoning.id).unwrap_or_default();
                if text.is_empty() {
                    text = reasoning
                        .summary_text
                        .iter()
                        .chain(reasoning.raw_content.iter())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n\n");
                }
                if !text.trim().is_empty() {
                    let (header, content) = history_cell::split_reasoning_summary_parts(&[text]);
                    let cwd = self.cwd.clone();
                    self.push_cell(history_cell::ReasoningSummaryCell::new(
                        header, content, &cwd, /*transcript_only*/ false,
                    ));
                }
            }
            _ => {}
        }
    }

    fn on_ui_render(&mut self, event: UiRenderEvent) {
        let node = match ether_ui_spec::parse(&event.spec) {
            Ok(node) => node,
            Err(err) => {
                self.push_cell(history_cell::new_error_event(format!("render_ui: {err}")));
                return;
            }
        };
        // Text streamed before the view goes above it.
        self.finish_stream();
        match self
            .ui_views
            .iter_mut()
            .find(|view| view.view_id() == event.view_id)
        {
            Some(view) => view.set_node(node),
            None => self
                .ui_views
                .push(UiSpecCell::new(event.view_id, node, &self.cwd)),
        }
    }

    /// Moves the live views into history, ahead of whatever is written next.
    fn commit_views(&mut self) {
        for view in self.ui_views.drain(..) {
            self.pending_history.push(Box::new(view));
        }
    }

    /// Answer text: through the HTML fence filter when views can be shown, else straight to
    /// the stream (HTML blocks then stay code).
    fn on_answer_text(&mut self, text: &str) {
        self.answer_open = true;
        if self.images.is_none() {
            self.on_text_delta(text);
            return;
        }
        for output in self.fence.push(text) {
            self.on_fence_output(output);
        }
    }

    fn on_fence_output(&mut self, output: FenceOutput) {
        match output {
            FenceOutput::Text(text) if self.pending.is_empty() => self.on_text_delta(&text),
            FenceOutput::Text(text) => self.pending.push_back(Pending::Text(text)),
            FenceOutput::Block { html, source } => {
                let Some(images) = self.images.as_ref() else {
                    self.on_text_delta(&source);
                    return;
                };
                let seq = self.next_view;
                self.next_view += 1;
                let max_cols = self
                    .width
                    .saturating_sub(STREAM_RESERVED_COLS as u16)
                    .max(10);
                let chrome = images.chrome.clone();
                let cell = images.cell;
                let style = images.style.clone();
                let tx = self.image_tx.clone();
                tokio::task::spawn_blocking(move || {
                    let result = html_image::render_html(&chrome, &html, max_cols, cell, &style);
                    let _ = tx.send((seq, result));
                });
                if self.pending.is_empty() {
                    // Text before the view goes above it.
                    self.finish_stream();
                }
                self.pending.push_back(Pending::View {
                    seq,
                    source,
                    result: None,
                });
            }
        }
    }

    fn on_image(&mut self, (seq, result): ImageResult) {
        for item in self.pending.iter_mut() {
            if let Pending::View {
                seq: view_seq,
                result: slot,
                ..
            } = item
                && *view_seq == seq
            {
                *slot = Some(result);
                break;
            }
        }
        // Show everything that is ready, in answer order.
        while let Some(front) = self.pending.front() {
            if matches!(front, Pending::View { result: None, .. }) {
                break;
            }
            match self.pending.pop_front() {
                Some(Pending::Text(text)) => self.on_text_delta(&text),
                Some(Pending::View {
                    source,
                    result: Some(result),
                    ..
                }) => self.show_view(source, result),
                _ => {}
            }
        }
        if self.pending.is_empty() && !self.answer_open {
            self.finish_stream();
        }
    }

    fn show_view(&mut self, source: String, result: Result<RenderedImage, String>) {
        match result {
            Ok(image) => {
                let id = html_image::new_image_id();
                self.pending_raw.push(html_image::transmit(
                    &image.png,
                    id,
                    image.cols,
                    image.rows,
                ));
                self.push_cell(ImageCell::new(id, &image));
            }
            Err(err) => {
                self.push_cell(history_cell::new_warning_event(format!(
                    "html view not rendered ({err}); showing its source"
                )));
                self.on_text_delta(&source);
                self.finish_stream();
            }
        }
    }

    fn on_text_delta(&mut self, delta: &str) {
        if self.stream.is_none() {
            self.commit_views();
            self.stream = Some(StreamController::new_with_inline_visualizations(
                self.stream_width(),
                &self.cwd,
                HistoryRenderMode::Rich,
                None,
            ));
        }
        if let Some(stream) = self.stream.as_mut() {
            stream.push(delta);
        }
    }

    fn finish_stream(&mut self) {
        if let Some(mut stream) = self.stream.take() {
            let (cell, _source) = stream.finalize();
            if let Some(cell) = cell {
                self.pending_history.push(cell);
            }
        }
    }

    fn on_tick(&mut self) {
        if let Some(stream) = self.stream.as_mut() {
            let (cell, _idle) = stream.on_commit_tick();
            if let Some(cell) = cell {
                self.pending_history.push(cell);
            }
        }
    }

    fn on_exec_begin(&mut self, begin: ExecCommandBeginEvent) {
        self.finish_stream();
        let cell = new_active_exec_command(
            begin.call_id.clone(),
            begin.command,
            begin.parsed_cmd,
            begin.source,
            begin.interaction_input,
            /*animations_enabled*/ true,
        );
        self.exec_cells.insert(begin.call_id, cell);
    }

    fn on_exec_end(&mut self, end: ExecCommandEndEvent) {
        let mut cell = match self.exec_cells.remove(&end.call_id) {
            Some(cell) => cell,
            None => new_active_exec_command(
                end.call_id.clone(),
                end.command.clone(),
                end.parsed_cmd.clone(),
                end.source,
                end.interaction_input.clone(),
                /*animations_enabled*/ true,
            ),
        };
        cell.complete_call(
            &end.call_id,
            CommandOutput::new(end.exit_code, end.aggregated_output),
            end.duration,
        );
        self.commit_views();
        self.pending_history.push(Box::new(cell));
    }

    fn end_turn(&mut self, note: Option<&str>) {
        if note.is_some() {
            // An interrupted turn ends its commands without `ExecCommandEnd`.
            for cell in self.exec_cells.values_mut() {
                cell.mark_failed();
            }
        }
        self.flush_live_cells();
        if let Some(note) = note {
            self.push_cell(history_cell::new_error_event(note.to_string()));
        }
        self.approval = None;
        self.turn_started = None;
    }

    /// Moves everything still live into history.
    fn flush_live_cells(&mut self) {
        self.finish_stream();
        self.commit_views();
        let mut cells: Vec<(String, ExecCell)> = self.exec_cells.drain().collect();
        cells.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, cell) in cells {
            self.pending_history.push(Box::new(cell));
        }
        for (_, text) in std::mem::take(&mut self.reasoning) {
            if !text.trim().is_empty() {
                let (header, content) = history_cell::split_reasoning_summary_parts(&[text]);
                let cwd = self.cwd.clone();
                self.pending_history
                    .push(Box::new(history_cell::ReasoningSummaryCell::new(
                        header, content, &cwd, /*transcript_only*/ false,
                    )));
            }
        }
    }

    fn push_cell(&mut self, cell: impl HistoryCell + 'static) {
        // History is ordered: finish the streamed answer first so the cell lands after it.
        self.finish_stream();
        self.commit_views();
        self.pending_history.push(Box::new(cell));
    }

    // ---- drawing --------------------------------------------------------------------------

    fn draw(&mut self, tui: &mut Tui) -> anyhow::Result<()> {
        let screen = tui.screen_size()?;
        self.width = screen.width;
        let width = screen.width;
        for raw in std::mem::take(&mut self.pending_raw) {
            tui.write_raw(&raw);
        }
        for cell in std::mem::take(&mut self.pending_history) {
            let mut lines = cell.display_hyperlink_lines(width);
            if lines.is_empty() {
                continue;
            }
            // A blank row between history cells, as in Codex.
            lines.insert(0, HyperlinkLine::from(""));
            tui.insert_history(lines);
        }

        let live = self.live_lines(width);
        let bottom = self.bottom_lines(width);
        let composer_height = if self.approval.is_some() {
            0
        } else {
            self.composer
                .desired_height(width.saturating_sub(COMPOSER_PREFIX_WIDTH))
                .clamp(1, 8)
        };
        let max_live = screen
            .height
            .saturating_sub(composer_height + bottom.len() as u16 + 2);
        let live_height = (live.len() as u16).min(max_live);
        let height = live_height + bottom.len() as u16 + composer_height + 1;

        let composer = &self.composer;
        let composer_state = &mut self.composer_state;
        tui.draw(height, |frame| {
            let area = frame.area();
            let buf = frame.buffer_mut();
            let mut y = area.y;
            if live_height > 0 {
                let skip = live.len() - usize::from(live_height);
                let text: Vec<Line<'static>> = live[skip..].to_vec();
                Paragraph::new(text).render(Rect::new(area.x, y, area.width, live_height), buf);
                y += live_height;
            }
            // Blank spacer row.
            y += 1;
            let bottom_height = bottom.len() as u16;
            Paragraph::new(bottom.clone())
                .wrap(Wrap { trim: false })
                .render(Rect::new(area.x, y, area.width, bottom_height), buf);
            y += bottom_height;
            if composer_height > 0 {
                let prefix_width = COMPOSER_PREFIX_WIDTH;
                Line::from(COMPOSER_PREFIX.bold().cyan())
                    .render(Rect::new(area.x, y, prefix_width, 1), buf);
                let text_area = Rect::new(
                    area.x + prefix_width,
                    y,
                    area.width.saturating_sub(prefix_width),
                    composer_height,
                );
                StatefulWidgetRef::render_ref(&composer, text_area, buf, composer_state);
                if let Some((cx, cy)) = composer.cursor_pos_with_state(text_area, *composer_state) {
                    frame.set_cursor_position((cx, cy));
                }
            }
        })?;
        Ok(())
    }

    /// Lines still changing: the streaming answer's tail, live views and running commands.
    fn live_lines(&self, width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if let Some(stream) = self.stream.as_ref() {
            let tail = stream.current_tail_lines();
            if !tail.is_empty() {
                lines.extend(prefix_tail(
                    visible_lines(tail),
                    stream.tail_starts_stream(),
                ));
            }
        }
        for view in &self.ui_views {
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            lines.extend(view.display_lines(width));
        }
        let mut cells: Vec<&ExecCell> = self.exec_cells.values().collect();
        cells.sort_by_key(|cell| cell.active_start_time());
        for cell in cells {
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            lines.extend(cell.display_lines(width));
        }
        lines
    }

    /// The status line plus either the approval prompt or the key hints.
    fn bottom_lines(&self, _width: u16) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if let Some(approval) = self.approval.as_ref() {
            lines.push(Line::from(approval.question.clone().bold()));
            if let Some(reason) = approval.reason.as_ref() {
                lines.push(Line::from(format!("  {reason}").dim()));
            }
            lines.push(Line::from(vec![
                "  y".bold(),
                " yes  ".into(),
                "a".bold(),
                " yes for this session  ".into(),
                "n".bold(),
                " no".into(),
            ]));
            return lines;
        }
        lines.push(self.status_line());
        lines
    }

    fn status_line(&self) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = Vec::new();
        if let Some(started) = self.turn_started {
            let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let frame = (started.elapsed().as_millis() / 80) as usize % spinner.len();
            spans.push(spinner[frame].cyan());
            spans.push(format!(" working {}s", started.elapsed().as_secs()).into());
            spans.push(" · esc to interrupt".dim());
            spans.push(" · ".dim());
        }
        spans.push(self.model.clone().dim());
        if self.turn_started.is_none() {
            spans.push(" · ".dim());
            spans.push(short_path(&self.cwd).dim());
        }
        if let Some(info) = self.token_info.as_ref() {
            let last = &info.last_token_usage;
            spans.push(" · ".dim());
            spans.push(
                format!(
                    "{} in ({} cached) · {} out",
                    last.input_tokens, last.cached_input_tokens, last.output_tokens
                )
                .dim(),
            );
        }
        Line::from(spans)
    }
}

/// The live tail of a streaming answer, with the `• ` marker on its first line when it starts
/// the message.
fn prefix_tail(lines: Vec<Line<'static>>, starts_stream: bool) -> Vec<Line<'static>> {
    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let prefix: Span<'static> = if index == 0 && starts_stream {
                "• ".dim()
            } else {
                "  ".into()
            };
            let mut spans = vec![prefix];
            spans.extend(line.spans);
            Line::from(spans).style(line.style)
        })
        .collect()
}

fn convert_changes(
    changes: HashMap<PathBuf, FileChange>,
) -> HashMap<PathBuf, diff_model::FileChange> {
    changes
        .into_iter()
        .map(|(path, change)| {
            let change = match change {
                FileChange::Add { content } => diff_model::FileChange::Add { content },
                FileChange::Delete { content } => diff_model::FileChange::Delete { content },
                FileChange::Update {
                    unified_diff,
                    move_path,
                } => diff_model::FileChange::Update {
                    unified_diff,
                    move_path,
                },
            };
            (path, change)
        })
        .collect()
}

fn describe_changes(changes: &HashMap<PathBuf, FileChange>) -> String {
    let mut parts: Vec<String> = changes
        .keys()
        .map(|path| path.display().to_string())
        .collect();
    parts.sort();
    parts.join(", ")
}

fn shlex_join(command: &[String]) -> String {
    shlex::try_join(command.iter().map(String::as_str)).unwrap_or_else(|_| command.join(" "))
}

fn short_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}
