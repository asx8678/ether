// New in ether: per-session state and services, in the role of Codex's `Session` (codex-rs/core/src/session/session.rs @ 806d973) without its global services.
//! A [`Session`] is shared by the submission loop and the running turn. It owns everything one
//! agent needs (configuration, model client, history, the event sender), so several sessions can
//! run in one process.

use crate::config::EngineConfig;
use crate::history::History;
use crate::tools::ToolRouter;
use codex_protocol::ThreadId;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::TokenCountEvent;
use codex_protocol::protocol::TokenUsage;
use ether_llm::ModelClient;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub(crate) struct Session {
    pub(crate) thread_id: ThreadId,
    pub(crate) config: EngineConfig,
    pub(crate) model: Arc<dyn ModelClient>,
    pub(crate) tools: ToolRouter,
    tx_event: mpsc::Sender<Event>,
    state: Mutex<SessionState>,
}

#[derive(Default)]
struct SessionState {
    history: History,
    active_turn: Option<ActiveTurn>,
    turn_counter: u64,
}

/// The turn currently running, if any.
pub(crate) struct ActiveTurn {
    pub(crate) turn_id: String,
    pub(crate) cancel: CancellationToken,
    /// Input that arrived while the turn was running, recorded before the next model request.
    pub(crate) pending_input: Vec<ResponseItem>,
}

impl Session {
    pub(crate) fn new(
        config: EngineConfig,
        model: Arc<dyn ModelClient>,
        tx_event: mpsc::Sender<Event>,
    ) -> Self {
        Self {
            thread_id: ThreadId::new(),
            config,
            model,
            tools: ToolRouter::new(),
            tx_event,
            state: Mutex::new(SessionState::default()),
        }
    }

    /// Sends an event to the client. A closed receiver means the client is gone; events are
    /// then dropped.
    pub(crate) async fn send_event(&self, id: &str, msg: EventMsg) {
        let event = Event {
            id: id.to_string(),
            msg,
        };
        let _ = self.tx_event.send(event).await;
    }

    pub(crate) async fn record_items(&self, items: &[ResponseItem]) {
        let policy = self.config.model.truncation_policy;
        self.state.lock().await.history.record(items, policy);
    }

    pub(crate) async fn prompt_input(&self) -> Vec<ResponseItem> {
        self.state.lock().await.history.for_prompt()
    }

    /// Closes tool calls left open by an interrupted turn and returns the outputs added.
    pub(crate) async fn close_open_calls(&self) -> Vec<ResponseItem> {
        self.state.lock().await.history.close_open_calls()
    }

    pub(crate) async fn update_token_usage(&self, usage: &TokenUsage) -> TokenCountEvent {
        let mut state = self.state.lock().await;
        state
            .history
            .update_token_info(usage, self.config.model.context_window);
        TokenCountEvent {
            info: state.history.token_info().cloned(),
            rate_limits: None,
        }
    }

    /// Starts tracking a new turn and returns its id and cancellation token, or `None` when a
    /// turn is already running.
    pub(crate) async fn begin_turn(&self) -> Option<(String, CancellationToken)> {
        let mut state = self.state.lock().await;
        if state.active_turn.is_some() {
            return None;
        }
        state.turn_counter += 1;
        let turn_id = state.turn_counter.to_string();
        let cancel = CancellationToken::new();
        state.active_turn = Some(ActiveTurn {
            turn_id: turn_id.clone(),
            cancel: cancel.clone(),
            pending_input: Vec::new(),
        });
        Some((turn_id, cancel))
    }

    pub(crate) async fn end_turn(&self, turn_id: &str) {
        let mut state = self.state.lock().await;
        if state
            .active_turn
            .as_ref()
            .is_some_and(|turn| turn.turn_id == turn_id)
        {
            state.active_turn = None;
        }
    }

    /// Ends the turn unless input is waiting for it; returns that input instead. Doing both
    /// under one lock means input submitted while a turn finishes is never lost.
    pub(crate) async fn end_turn_unless_pending(&self, turn_id: &str) -> Vec<ResponseItem> {
        let mut state = self.state.lock().await;
        let Some(turn) = state
            .active_turn
            .as_mut()
            .filter(|turn| turn.turn_id == turn_id)
        else {
            return Vec::new();
        };
        let pending = std::mem::take(&mut turn.pending_input);
        if pending.is_empty() {
            state.active_turn = None;
        }
        pending
    }

    /// Queues input for the running turn. Returns its id, or `None` when no turn is running.
    pub(crate) async fn steer(&self, items: Vec<ResponseItem>) -> Option<String> {
        let mut state = self.state.lock().await;
        let turn = state.active_turn.as_mut()?;
        turn.pending_input.extend(items);
        Some(turn.turn_id.clone())
    }

    pub(crate) async fn take_pending_input(&self) -> Vec<ResponseItem> {
        let mut state = self.state.lock().await;
        state
            .active_turn
            .as_mut()
            .map(|turn| std::mem::take(&mut turn.pending_input))
            .unwrap_or_default()
    }

    pub(crate) async fn has_pending_input(&self) -> bool {
        let state = self.state.lock().await;
        state
            .active_turn
            .as_ref()
            .is_some_and(|turn| !turn.pending_input.is_empty())
    }

    /// Cancels the running turn, if any. The turn emits `TurnAborted` when it stops.
    pub(crate) async fn interrupt(&self) -> bool {
        let state = self.state.lock().await;
        match state.active_turn.as_ref() {
            Some(turn) => {
                turn.cancel.cancel();
                true
            }
            None => false,
        }
    }
}
