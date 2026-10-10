// New in ether: plain-text command line front end for the engine (replaced by the TUI in Phase 5).
//! `ether [PROMPT]` runs one turn and exits; without a prompt it reads one prompt per line.
//! Ctrl-C interrupts the running turn; Ctrl-D ends the session.

use anyhow::Context;
use clap::Parser;
use clap::ValueEnum;
use codex_protocol::approvals::ExecApprovalRequestEvent;
use codex_protocol::config_types::SandboxMode;
use codex_protocol::items::AgentMessageContent;
use codex_protocol::items::TurnItem;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
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

    let mut engine = Engine::spawn(config, model);
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
    let _ = engine.submit(Op::Shutdown);
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
        if let EventMsg::ExecApprovalRequest(request) = &event.msg {
            printer.end_stream();
            let decision = ask_exec_approval(printer, request).await;
            let _ = engine.submit(Op::ExecApproval {
                id: request.effective_approval_id(),
                turn_id: Some(request.turn_id.clone()),
                decision,
            });
            continue;
        }
        if printer.print(&event.msg) {
            return;
        }
    }
}

/// Asks on the terminal whether to run a command. Without a terminal the command is declined.
async fn ask_exec_approval(
    printer: &Printer,
    request: &ExecApprovalRequestEvent,
) -> ReviewDecision {
    let command = shlex_join(&request.command);
    eprintln!(
        "{}",
        printer.paint(&format!("approve? $ {command}"), "1;33")
    );
    if let Some(reason) = &request.reason {
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
    let answer = tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).map(|_| line)
    })
    .await;
    match answer {
        Ok(Ok(line)) => match line.trim() {
            "y" | "yes" => ReviewDecision::Approved,
            "a" => ReviewDecision::ApprovedForSession,
            _ => ReviewDecision::Abort,
        },
        _ => ReviewDecision::Abort,
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
                            "ether · {} via {} · {}",
                            configured.model,
                            configured.model_provider_id,
                            configured.cwd.display()
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
