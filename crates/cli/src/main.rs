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
use codex_protocol::protocol::Op;
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
use ether_llm::ResponsesModelClient;
use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;
use tokio::sync::oneshot;

#[derive(Debug, Parser)]
#[command(name = "ether", version, about = "A minimal coding agent")]
struct Cli {
    /// Prompt to run. Without one, ether reads prompts from standard input, one per line.
    prompt: Option<String>,

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

    /// With --replay, append each request's prompt as a JSON line to FILE.
    #[arg(long, value_name = "FILE", requires = "replay")]
    replay_log: Option<PathBuf>,
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
    let config = ether_engine::load_config(ConfigOverrides {
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

    let model: Arc<dyn ModelClient> = match cli.replay.as_deref() {
        Some(path) => {
            let mut client = ReplayModelClient::from_file(path)?;
            if let Some(log) = cli.replay_log.as_deref() {
                client = client
                    .with_request_log(log)
                    .with_context(|| format!("cannot open {}", log.display()))?;
            }
            Arc::new(client)
        }
        None => Arc::new(ResponsesModelClient::from_provider(&config.model_provider)?),
    };

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
    let mut engine = match resume_path {
        Some(path) => Engine::resume(config, model, &path)
            .await
            .with_context(|| format!("cannot resume {}", path.display()))?,
        None => Engine::spawn(config, model),
    };
    let mut printer = Printer::new();
    // The first event is `SessionConfigured`; show it before the first prompt.
    if let Some(event) = engine.next_event().await {
        printer.print(&event.msg);
    }

    match cli.prompt {
        Some(prompt) => {
            submit_prompt(&engine, prompt)?;
            run_until_turn_ends(&mut engine, &mut printer).await;
        }
        None => interactive(&mut engine, &mut printer).await?,
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

async fn interactive(engine: &mut Engine, printer: &mut Printer) -> anyhow::Result<()> {
    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    loop {
        printer.prompt();
        let Some(line) = lines.next_line().await? else {
            return Ok(());
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        submit_prompt(engine, line.to_string())?;
        run_until_turn_ends(engine, printer).await;
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
async fn run_until_turn_ends(engine: &mut Engine, printer: &mut Printer) {
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
        if printer.print(&event.msg) {
            return;
        }
    }
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
