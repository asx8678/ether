// New in ether: plain-text command line front end for the engine (replaced by the TUI in Phase 5).
//! `ether [PROMPT]` runs one turn and exits; without a prompt it reads one prompt per line.
//! Ctrl-C interrupts the running turn; Ctrl-D ends the session. `--resume` / `--last` continue a
//! recorded session.

use anyhow::Context;
use clap::Parser;
use clap::ValueEnum;
use codex_protocol::config_types::SandboxMode;
use codex_protocol::items::AgentMessageContent;
use codex_protocol::items::TurnItem;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::FileChange;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::PatchApplyStatus;
use codex_protocol::protocol::ReviewDecision;
use codex_protocol::protocol::ThreadSettingsOverrides;
use codex_protocol::protocol::TokenCountEvent;
use codex_protocol::turn_input::TurnInput;
use codex_protocol::turn_input::TurnInputMode;
use codex_protocol::turn_input::TurnInputRequest;
use codex_protocol::turn_input::TurnStartOptions;
use codex_protocol::user_input::UserInput;
use ether_engine::ConfigOverrides;
use ether_engine::Engine;
use ether_llm::ModelClient;
use ether_llm::ReplayModelClient;
use ether_llm::RequestLog;
use ether_llm::client_for_provider;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::io::IsTerminal;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::sync::oneshot;

/// Width of `render_ui` views in plain mode.
const PLAIN_VIEW_WIDTH: u16 = 80;

#[derive(Debug, Parser)]
#[command(name = "ether", version, about = "A minimal coding agent")]
struct Cli {
    /// Prompt to run. In the TUI it is submitted first; in plain mode ether runs it and exits.
    /// Without one, plain mode reads prompts from standard input, one per line.
    prompt: Option<String>,

    /// Use plain text even on a terminal.
    #[arg(long)]
    no_tui: bool,

    /// Model slug.
    #[arg(short, long)]
    model: Option<String>,

    /// Model provider id from config (`[model_providers.<id>]`), or a built-in one.
    #[arg(short = 'p', long = "provider")]
    provider: Option<String>,

    /// When to ask before running commands.
    #[arg(short, long, value_enum)]
    approval: Option<ApprovalArg>,

    /// What tool calls may touch.
    #[arg(short, long, value_enum)]
    sandbox: Option<SandboxArg>,

    /// Working directory for the session.
    #[arg(short = 'C', long = "cd")]
    cwd: Option<PathBuf>,

    /// Answer model requests from a file of recorded Responses API events instead of a provider.
    #[arg(long, value_name = "FILE")]
    replay: Option<PathBuf>,

    /// Resume a recorded session, given its id or rollout file.
    #[arg(long, value_name = "SESSION_ID_OR_PATH", conflicts_with = "last")]
    resume: Option<String>,

    /// Resume the newest session recorded in the working directory.
    #[arg(long)]
    last: bool,

    /// Append each model request's body, as sent on the provider's wire API, as a JSON line to FILE.
    #[arg(long, value_name = "FILE", alias = "replay-log")]
    request_log: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ApprovalArg {
    Untrusted,
    OnRequest,
    Never,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SandboxArg {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging();
    let cli = Cli::parse();
    let mut config = ether_engine::load_config(ConfigOverrides {
        model: cli.model.clone(),
        model_provider: cli.provider.clone(),
        approval_policy: cli.approval.map(|approval| match approval {
            ApprovalArg::Untrusted => AskForApproval::UnlessTrusted,
            ApprovalArg::OnRequest => AskForApproval::OnRequest,
            ApprovalArg::Never => AskForApproval::Never,
        }),
        sandbox_mode: cli.sandbox.map(|sandbox| match sandbox {
            SandboxArg::ReadOnly => SandboxMode::ReadOnly,
            SandboxArg::WorkspaceWrite => SandboxMode::WorkspaceWrite,
            SandboxArg::DangerFullAccess => SandboxMode::DangerFullAccess,
        }),
        cwd: cli.cwd.clone(),
    })?;

    let mut model: Arc<dyn ModelClient> = match cli.replay.as_deref() {
        Some(path) => Arc::new(ReplayModelClient::from_file(path)?),
        None => client_for_provider(&config.model_provider)?,
    };
    if let Some(log) = cli.request_log.as_deref() {
        // The body is logged in the configured provider's wire format, also under --replay.
        model = Arc::new(
            RequestLog::new(model, config.model_provider.wire_api, log)
                .with_context(|| format!("cannot open {}", log.display()))?,
        );
    }

    let resume_path = match (cli.resume.as_deref(), cli.last) {
        (Some(id_or_path), _) => Some(
            ether_engine::find_session(&config, id_or_path)
                .await?
                .with_context(|| format!("no recorded session matches {id_or_path}"))?,
        ),
        (None, true) => Some(
            ether_engine::find_latest_session(&config)
                .await?
                .context("no recorded session in this directory")?,
        ),
        (None, false) => None,
    };
    // The TUI on a terminal; plain text for pipes, files and `--no-tui`.
    let use_tui = !cli.no_tui && std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if use_tui && ether_tui::views_available() {
        // The TUI draws ```view blocks; tell the model (render_ui then defaults to off).
        config.display_instructions =
            Some(ether_tui::view_instructions(
                ether_tui::terminal_is_dark(),
                ether_tui::view_width_px(),
            ));
    }
    let ether_home = config.ether_home.clone();
    let host = Arc::new(Sessions {
        config: config.clone(),
        model: model.clone(),
    });
    let mut engine = match resume_path {
        Some(path) => Engine::resume(config, model, &path)
            .await
            .with_context(|| format!("cannot resume {}", path.display()))?,
        None => Engine::spawn(config, model),
    };
    if use_tui {
        return ether_tui::run(ether_tui::Launch {
            engine,
            initial_prompt: cli.prompt,
            ether_home,
            sessions: host,
        })
        .await;
    }

    let mut printer = Printer::new();
    // The first event is `SessionConfigured`; show it before the first prompt.
    if let Some(event) = engine.next_event().await {
        printer.print(&event.msg);
    }

    // One reader for all of stdin: prompts and approval answers come from the same lines.
    let mut input = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    match cli.prompt {
        Some(prompt) => {
            submit_prompt(&engine, prompt)?;
            run_until_turn_ends(&mut engine, &mut printer, &mut input).await;
        }
        None => interactive(&mut engine, &mut printer, &mut input).await?,
    }
    // Wait for `ShutdownComplete` so the session's rollout is written out before exiting.
    if engine.submit(Op::Shutdown).is_ok() {
        while let Some(event) = engine.next_event().await {
            if matches!(event.msg, EventMsg::ShutdownComplete) {
                break;
            }
        }
    }
    Ok(())
}

type Input = tokio::io::Lines<tokio::io::BufReader<tokio::io::Stdin>>;

async fn interactive(
    engine: &mut Engine,
    printer: &mut Printer,
    input: &mut Input,
) -> anyhow::Result<()> {
    loop {
        printer.prompt();
        let Some(line) = input.next_line().await? else {
            return Ok(());
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        submit_prompt(engine, line.to_string())?;
        run_until_turn_ends(engine, printer, input).await;
    }
}

fn submit_prompt(engine: &Engine, prompt: String) -> anyhow::Result<()> {
    let (reply, _reply_rx) = oneshot::channel();
    engine.submit(Op::TurnInput {
        request: Box::new(TurnInputRequest {
            input: TurnInput::UserInput {
                content: vec![UserInput::Text {
                    text: prompt,
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

/// Prints events until the current turn completes or is aborted. Ctrl-C interrupts it.
async fn run_until_turn_ends(engine: &mut Engine, printer: &mut Printer, input: &mut Input) {
    let mut interrupted = false;
    loop {
        let event = tokio::select! {
            event = engine.next_event() => event,
            _ = tokio::signal::ctrl_c(), if !interrupted => {
                interrupted = true;
                let _ = engine.submit(Op::Interrupt);
                continue;
            }
        };
        let Some(event) = event else {
            return;
        };
        match &event.msg {
            EventMsg::ExecApprovalRequest(request) => {
                printer.end_stream();
                let question = format!("approve? $ {}", shlex_join(&request.command));
                let decision =
                    ask_approval(printer, input, &question, request.reason.as_deref()).await;
                let _ = engine.submit(Op::ExecApproval {
                    id: request.effective_approval_id(),
                    turn_id: Some(request.turn_id.clone()),
                    decision,
                });
            }
            EventMsg::ApplyPatchApprovalRequest(request) => {
                printer.end_stream();
                let question = format!("approve patch? {}", describe_changes(&request.changes));
                let decision =
                    ask_approval(printer, input, &question, request.reason.as_deref()).await;
                let _ = engine.submit(Op::PatchApproval {
                    id: request.call_id.clone(),
                    decision,
                });
            }
            msg => {
                if printer.print(msg) {
                    return;
                }
            }
        }
    }
}

/// Asks on the terminal. Without a terminal, or at end of input, the request is declined.
async fn ask_approval(
    printer: &Printer,
    input: &mut Input,
    question: &str,
    reason: Option<&str>,
) -> ReviewDecision {
    eprintln!("{}", printer.paint(question, "1;33"));
    if let Some(reason) = reason {
        eprintln!("{}", printer.paint(&format!("  {reason}"), "2"));
    }
    if !std::io::stdin().is_terminal() {
        eprintln!(
            "{}",
            printer.paint("  no terminal to ask on; declining", "2")
        );
        return ReviewDecision::Abort;
    }
    eprint!(
        "{}",
        printer.paint("  [y] yes  [a] yes for this session  [n] no > ", "1")
    );
    let _ = std::io::stderr().flush();
    match input.next_line().await {
        Ok(Some(line)) => match line.trim() {
            "y" | "yes" => ReviewDecision::Approved,
            "a" => ReviewDecision::ApprovedForSession,
            _ => ReviewDecision::Abort,
        },
        _ => ReviewDecision::Abort,
    }
}

/// One-line summary of a patch: `A new.rs, M lib.rs, D old.rs`.
fn describe_changes(changes: &HashMap<PathBuf, FileChange>) -> String {
    let mut parts: Vec<String> = changes
        .iter()
        .map(|(path, change)| {
            let kind = match change {
                FileChange::Add { .. } => "A",
                FileChange::Delete { .. } => "D",
                FileChange::Update { .. } => "M",
            };
            format!("{kind} {}", path.display())
        })
        .collect();
    parts.sort();
    parts.join(", ")
}

/// Renders events as plain text. Assistant text goes to stdout; everything else to stderr.
struct Printer {
    color: bool,
    /// Kind of streamed output on the current line, so each stream starts on a fresh line.
    streaming: Option<Stream>,
    token_count: Option<TokenCountEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stream {
    Text,
    Reasoning,
}

impl Printer {
    fn new() -> Self {
        Self {
            color: std::io::stderr().is_terminal(),
            streaming: None,
            token_count: None,
        }
    }

    fn prompt(&self) {
        if std::io::stdin().is_terminal() {
            eprint!("{}", self.paint("> ", "1"));
            let _ = std::io::stderr().flush();
        }
    }

    /// Returns `true` when the event ends the current turn.
    fn print(&mut self, msg: &EventMsg) -> bool {
        match msg {
            EventMsg::SessionConfigured(configured) => {
                eprintln!(
                    "{}",
                    self.paint(
                        &format!(
                            "ether · {} via {} · {} · session {}",
                            configured.model,
                            configured.model_provider_id,
                            configured.cwd.display(),
                            configured.thread_id
                        ),
                        "2"
                    )
                );
            }
            EventMsg::AgentMessageContentDelta(delta) => {
                self.start_stream(Stream::Text);
                print!("{}", delta.delta);
                let _ = std::io::stdout().flush();
            }
            EventMsg::ReasoningContentDelta(delta) => {
                self.start_stream(Stream::Reasoning);
                eprint!("{}", self.paint(&delta.delta, "2"));
            }
            EventMsg::ReasoningRawContentDelta(delta) => {
                self.start_stream(Stream::Reasoning);
                eprint!("{}", self.paint(&delta.delta, "2"));
            }
            EventMsg::ItemCompleted(completed) => {
                if let TurnItem::AgentMessage(message) = &completed.item
                    && self.streaming != Some(Stream::Text)
                {
                    // Nothing was streamed for this message; print it whole.
                    let text: String = message
                        .content
                        .iter()
                        .map(|part| match part {
                            AgentMessageContent::Text { text } => text.as_str(),
                        })
                        .collect();
                    self.end_stream();
                    println!("{text}");
                } else {
                    self.end_stream();
                }
            }
            EventMsg::ExecCommandBegin(begin) => {
                self.end_stream();
                let command = display_command(&begin.command);
                eprintln!("{}", self.paint(&format!("$ {command}"), "1"));
            }
            EventMsg::ExecCommandOutputDelta(delta) => {
                eprint!(
                    "{}",
                    self.paint(&String::from_utf8_lossy(&delta.chunk), "2")
                );
            }
            EventMsg::ExecCommandEnd(end) => {
                let status = format!(
                    "exit {} in {:.1}s",
                    end.exit_code,
                    end.duration.as_secs_f64()
                );
                let color = if end.exit_code == 0 { "2" } else { "31" };
                eprintln!("{}", self.paint(&status, color));
            }
            EventMsg::PatchApplyBegin(begin) => {
                self.end_stream();
                eprintln!(
                    "{}",
                    self.paint(
                        &format!("patch: {}", describe_changes(&begin.changes)),
                        "36"
                    )
                );
            }
            EventMsg::PatchApplyEnd(end) if !end.success => {
                self.end_stream();
                let message = if end.status == PatchApplyStatus::Declined {
                    "patch declined".to_string()
                } else {
                    let detail = if end.stderr.is_empty() {
                        &end.stdout
                    } else {
                        &end.stderr
                    };
                    format!("patch failed: {}", detail.trim())
                };
                eprintln!("{}", self.paint(&message, "31"));
            }
            EventMsg::UiRender(view) => {
                self.end_stream();
                match ether_tui::render_ui_plain(&view.spec, PLAIN_VIEW_WIDTH) {
                    Ok(lines) => {
                        for line in lines {
                            println!("{line}");
                        }
                    }
                    Err(err) => eprintln!("{}", self.paint(&format!("render_ui: {err}"), "31")),
                }
            }
            EventMsg::TokenCount(token_count) => {
                self.token_count = Some(token_count.clone());
            }
            EventMsg::StreamError(error) => {
                self.end_stream();
                eprintln!("{}", self.paint(&error.message, "33"));
            }
            EventMsg::Warning(warning) => {
                self.end_stream();
                eprintln!(
                    "{}",
                    self.paint(&format!("warning: {}", warning.message), "33")
                );
            }
            EventMsg::Error(error) => {
                self.end_stream();
                eprintln!("{}", self.paint(&format!("error: {}", error.message), "31"));
            }
            EventMsg::TurnComplete(_) => {
                self.end_stream();
                self.print_token_count();
                return true;
            }
            EventMsg::TurnAborted(_) => {
                self.end_stream();
                eprintln!("{}", self.paint("interrupted", "33"));
                return true;
            }
            EventMsg::ShutdownComplete => return true,
            _ => {}
        }
        false
    }

    fn start_stream(&mut self, stream: Stream) {
        if self.streaming != Some(stream) {
            self.end_stream();
            self.streaming = Some(stream);
        }
    }

    fn end_stream(&mut self) {
        match self.streaming.take() {
            Some(Stream::Text) => println!(),
            Some(Stream::Reasoning) => eprintln!(),
            None => {}
        }
    }

    fn print_token_count(&mut self) {
        let Some(info) = self.token_count.take().and_then(|count| count.info) else {
            return;
        };
        let last = &info.last_token_usage;
        eprintln!(
            "{}",
            self.paint(
                &format!(
                    "tokens: {} in ({} cached), {} out",
                    last.input_tokens, last.cached_input_tokens, last.output_tokens
                ),
                "2"
            )
        );
    }

    fn paint(&self, text: &str, sgr: &str) -> String {
        if self.color {
            format!("\x1b[{sgr}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("ETHER_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("error"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .try_init();
}

fn shlex_join(command: &[String]) -> String {
    shlex::try_join(command.iter().map(String::as_str)).unwrap_or_else(|_| command.join(" "))
}

/// The script of a `shell -lc SCRIPT` command, or the whole command line.
fn display_command(command: &[String]) -> String {
    match command {
        [_, flag, script] if flag == "-lc" || flag == "-c" => script.clone(),
        _ => shlex_join(command),
    }
}

/// Starts, resumes and lists sessions for the TUI's `/new` and `/resume`, with the
/// configuration and model client the CLI started with.
struct Sessions {
    config: ether_engine::EngineConfig,
    model: Arc<dyn ModelClient>,
}

impl ether_tui::SessionHost for Sessions {
    fn new_session(&self) -> futures::future::BoxFuture<'static, anyhow::Result<Engine>> {
        let engine = Engine::spawn(self.config.clone(), self.model.clone());
        Box::pin(async move { Ok(engine) })
    }

    fn resume_session(
        &self,
        path: PathBuf,
    ) -> futures::future::BoxFuture<'static, anyhow::Result<Engine>> {
        let (config, model) = (self.config.clone(), self.model.clone());
        Box::pin(async move {
            Engine::resume(config, model, &path)
                .await
                .with_context(|| format!("cannot resume {}", path.display()))
        })
    }

    fn list_sessions(
        &self,
    ) -> futures::future::BoxFuture<'static, anyhow::Result<Vec<ether_tui::SessionEntry>>> {
        let config = self.config.clone();
        Box::pin(async move {
            let sessions = ether_engine::list_sessions(&config, 30).await?;
            Ok(sessions
                .into_iter()
                .map(|session| ether_tui::SessionEntry {
                    path: session.path,
                    updated: session
                        .updated_at
                        .as_deref()
                        .and_then(|time| chrono::DateTime::parse_from_rfc3339(time).ok())
                        .map(|time| {
                            time.with_timezone(&chrono::Local)
                                .format("%Y-%m-%d %H:%M")
                                .to_string()
                        })
                        .unwrap_or_default(),
                    preview: session
                        .preview
                        .unwrap_or_default()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" "),
                })
                .collect())
        })
    }
}
