// New in ether: the engine handle (new or resumed sessions) and submission loop, in the role of Codex's submission loop (codex-rs/core/src/session/handlers.rs @ 806d973).
//! The engine is driven only through [`Op`] in and [`Event`] out. [`Engine::spawn`] starts one
//! session on the current Tokio runtime; dropping the handle or sending [`Op::Shutdown`] ends it.

use crate::config::EngineConfig;
use crate::history::History;
use crate::reconstruct::reconstruct_rollout;
use crate::rollout::Rollout;
use crate::rollout::permission_profile;
use crate::session::Session;
use crate::session::SessionInit;
use crate::turn::run_turn;
use codex_protocol::ThreadId;
use codex_protocol::error::CodexErr;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::ErrorEvent;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::SessionConfiguredEvent;
use codex_protocol::turn_input::NotSubmittedReason;
use codex_protocol::turn_input::TurnInput;
use codex_protocol::turn_input::TurnInputMode;
use codex_protocol::turn_input::TurnInputSubmission;
use codex_rollout::RolloutRecorder;
use ether_llm::ModelClient;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;

const EVENT_CHANNEL_CAPACITY: usize = 1024;

/// Id used for events that do not belong to a turn.
pub const SESSION_EVENT_ID: &str = "session";

/// Handle to one running engine session.
pub struct Engine {
    tx_op: mpsc::UnboundedSender<Op>,
    rx_event: mpsc::Receiver<Event>,
}

/// The engine has stopped and accepts no more operations.
#[derive(Debug, thiserror::Error)]
#[error("engine has shut down")]
pub struct EngineClosed;

impl Engine {
    /// Starts a session, recorded to a new rollout file. Its first event is `SessionConfigured`.
    pub fn spawn(config: EngineConfig, model: Arc<dyn ModelClient>) -> Self {
        Self::start(config, model, Start::New)
    }

    /// Resumes the session recorded at `rollout_path`: its history is rebuilt and new items are
    /// appended to the same file. Fails if the rollout cannot be read.
    pub async fn resume(
        config: EngineConfig,
        model: Arc<dyn ModelClient>,
        rollout_path: &Path,
    ) -> std::io::Result<Self> {
        let (items, _thread_id, _parse_errors) =
            RolloutRecorder::load_rollout_items(rollout_path).await?;
        let reconstruction = reconstruct_rollout(&items).ok_or_else(|| {
            std::io::Error::other(format!(
                "{} has no session metadata",
                rollout_path.display()
            ))
        })?;
        Ok(Self::start(
            config,
            model,
            Start::Resume {
                path: rollout_path.to_path_buf(),
                reconstruction: Box::new(reconstruction),
            },
        ))
    }

    fn start(config: EngineConfig, model: Arc<dyn ModelClient>, start: Start) -> Self {
        let (tx_op, rx_op) = mpsc::unbounded_channel();
        let (tx_event, rx_event) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
        tokio::spawn(async move {
            let init = session_init(&config, start).await;
            let session = Arc::new(Session::new(config, model, tx_event, init));
            // A resumed session may end in an interrupted turn; answer its open calls.
            session.close_open_calls().await;
            submission_loop(session, rx_op).await;
        });
        Self { tx_op, rx_event }
    }

    pub fn submit(&self, op: Op) -> Result<(), EngineClosed> {
        self.tx_op.send(op).map_err(|_| EngineClosed)
    }

    /// Returns the next event, or `None` once the session has ended.
    pub async fn next_event(&mut self) -> Option<Event> {
        self.rx_event.recv().await
    }
}

enum Start {
    New,
    Resume {
        path: std::path::PathBuf,
        reconstruction: Box<crate::reconstruct::RolloutReconstruction>,
    },
}

/// Opens the rollout. A rollout that cannot be opened leaves the session unrecorded.
async fn session_init(config: &EngineConfig, start: Start) -> SessionInit {
    match start {
        Start::New => {
            let thread_id = ThreadId::new();
            let rollout = Rollout::create(config, thread_id)
                .await
                .inspect_err(|err| tracing::warn!("session will not be recorded: {err}"))
                .ok();
            SessionInit {
                thread_id,
                history: History::default(),
                turn_counter: 0,
                rollout,
            }
        }
        Start::Resume {
            path,
            reconstruction,
        } => {
            let rollout = Rollout::resume(config, path, reconstruction.history_mode)
                .await
                .inspect_err(|err| tracing::warn!("resumed session will not be recorded: {err}"))
                .ok();
            SessionInit {
                thread_id: reconstruction.thread_id,
                history: reconstruction.history,
                turn_counter: reconstruction.last_turn_number,
                rollout,
            }
        }
    }
}

async fn submission_loop(sess: Arc<Session>, mut rx_op: mpsc::UnboundedReceiver<Op>) {
    sess.send_event(
        SESSION_EVENT_ID,
        EventMsg::SessionConfigured(session_configured(&sess)),
    )
    .await;

    while let Some(op) = rx_op.recv().await {
        match op {
            Op::TurnInput {
                request,
                mode,
                reply,
            } => {
                let result = submit_turn_input(&sess, request.input, mode).await;
                let _ = reply.send(result);
            }
            Op::Interrupt => {
                sess.interrupt().await;
            }
            Op::ExecApproval { id, decision, .. } | Op::PatchApproval { id, decision } => {
                if !sess.resolve_approval(&id, decision).await {
                    send_error(&sess, format!("no pending approval with id {id}")).await;
                }
            }
            Op::UserInputAnswer { id, response } => {
                if !sess.resolve_user_input(&id, response).await {
                    send_error(&sess, format!("no pending user input request with id {id}")).await;
                }
            }
            Op::Shutdown => {
                sess.interrupt().await;
                break;
            }
            other => {
                send_error(&sess, format!("unsupported operation: {}", other.kind())).await;
            }
        }
    }
    sess.shutdown_rollout().await;
    sess.send_event(SESSION_EVENT_ID, EventMsg::ShutdownComplete)
        .await;
}

async fn send_error(sess: &Session, message: String) {
    sess.send_event(
        SESSION_EVENT_ID,
        EventMsg::Error(ErrorEvent {
            message,
            codex_error_info: None,
            misalignment: None,
        }),
    )
    .await;
}

async fn submit_turn_input(
    sess: &Arc<Session>,
    input: TurnInput,
    mode: TurnInputMode,
) -> codex_protocol::error::Result<TurnInputSubmission> {
    let items = turn_input_items(input)?;
    if items.is_empty() {
        return Ok(not_submitted(NotSubmittedReason::EmptyInput));
    }
    match mode {
        TurnInputMode::StartOrSteer => match sess.steer(items.clone()).await {
            Some(turn_id) => Ok(steered(turn_id)),
            None => Ok(start_turn(sess, items).await),
        },
        TurnInputMode::StartIfIdle | TurnInputMode::ContinueIfIdle { .. } => {
            Ok(start_turn(sess, items).await)
        }
        TurnInputMode::Steer { expected_turn_id } => match sess.steer(items).await {
            Some(turn_id) if turn_id == expected_turn_id => Ok(steered(turn_id)),
            // The input was queued for a different turn; it still runs there.
            Some(turn_id) => Ok(steered(turn_id)),
            None => Ok(not_submitted(NotSubmittedReason::NoActiveTurn)),
        },
    }
}

async fn start_turn(sess: &Arc<Session>, items: Vec<ResponseItem>) -> TurnInputSubmission {
    let Some((turn_id, cancel)) = sess.begin_turn().await else {
        return not_submitted(NotSubmittedReason::NotIdle);
    };
    tokio::spawn(run_turn(Arc::clone(sess), turn_id.clone(), items, cancel));
    TurnInputSubmission::Started {
        root_turn_id: turn_id.clone(),
        turn_id,
    }
}

fn turn_input_items(input: TurnInput) -> codex_protocol::error::Result<Vec<ResponseItem>> {
    match input {
        TurnInput::UserInput { content, .. } => {
            if content.is_empty() {
                return Ok(Vec::new());
            }
            Ok(vec![ResponseItem::from(ResponseInputItem::from(content))])
        }
        TurnInput::ResponseItem(item) => Ok(vec![item]),
        TurnInput::AnnotatedResponseItem(annotated) => Ok(vec![annotated.item]),
        TurnInput::InterAgentCommunication(_) => Err(CodexErr::UnsupportedOperation(
            "inter-agent communication is not supported".to_string(),
        )),
    }
}

fn steered(turn_id: String) -> TurnInputSubmission {
    TurnInputSubmission::Steered {
        root_turn_id: turn_id.clone(),
        turn_id,
    }
}

fn not_submitted(reason: NotSubmittedReason) -> TurnInputSubmission {
    TurnInputSubmission::NotSubmitted { reason }
}

fn session_configured(sess: &Session) -> SessionConfiguredEvent {
    let config = &sess.config;
    let permission_profile = permission_profile(config.sandbox_mode);
    SessionConfiguredEvent {
        session_id: sess.thread_id.into(),
        thread_id: sess.thread_id,
        forked_from_id: None,
        parent_thread_id: None,
        thread_source: None,
        thread_name: None,
        model: config.model.slug.clone(),
        model_provider_id: config.model_provider_id.clone(),
        service_tier: None,
        approval_policy: config.approval_policy,
        approvals_reviewer: Default::default(),
        permission_profile,
        active_permission_profile: None,
        cwd: config.cwd.clone(),
        reasoning_effort: config.reasoning_effort.clone(),
        network_proxy: None,
        rollout_path: sess.rollout_path().map(Path::to_path_buf),
    }
}
