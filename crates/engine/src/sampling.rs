// New in ether: the per-response stream handling of Codex's `try_run_sampling_request` (codex-rs/core/src/session/turn.rs @ 806d973) and `handle_output_item_done` (core/src/stream_events_utils.rs), as a pure step function.
//! [`SamplingState::on_event`] takes one model stream event and returns the effects it causes;
//! it performs no I/O. The turn driver applies the effects (emits events, records history,
//! starts tools), which keeps the agent logic in a `step(state, event) -> effects` shape.

use crate::event_mapping::parse_turn_item;
use codex_protocol::ThreadId;
use codex_protocol::items::AgentMessageContent;
use codex_protocol::items::TurnItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::AgentMessageContentDeltaEvent;
use codex_protocol::protocol::AgentReasoningSectionBreakEvent;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ItemCompletedEvent;
use codex_protocol::protocol::ItemStartedEvent;
use codex_protocol::protocol::ReasoningContentDeltaEvent;
use codex_protocol::protocol::ReasoningRawContentDeltaEvent;
use codex_protocol::protocol::TokenUsage;
use ether_llm::ResponseEvent;
use tracing::warn;

/// Something the driver must do in response to a stream event.
#[derive(Debug)]
#[allow(
    clippy::large_enum_variant,
    reason = "effects are applied as soon as they are produced, never stored in bulk"
)]
pub(crate) enum Effect {
    /// Send this event to the client.
    Emit(EventMsg),
    /// Append this item to history (and the rollout).
    Record(ResponseItem),
    /// Execute this tool call; its output is recorded when it finishes.
    RunTool(ResponseItem),
}

/// How a response ended.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Completion {
    pub(crate) token_usage: Option<TokenUsage>,
}

/// State of one model response while it streams.
#[derive(Debug)]
pub(crate) struct SamplingState {
    thread_id: ThreadId,
    turn_id: String,
    active_item: Option<TurnItem>,
    needs_follow_up: bool,
    last_agent_message: Option<String>,
    completion: Option<Completion>,
}

impl SamplingState {
    pub(crate) fn new(thread_id: ThreadId, turn_id: String) -> Self {
        Self {
            thread_id,
            turn_id,
            active_item: None,
            needs_follow_up: false,
            last_agent_message: None,
            completion: None,
        }
    }

    /// Whether the model must be sampled again (it called tools, or did not end its turn).
    pub(crate) fn needs_follow_up(&self) -> bool {
        self.needs_follow_up
    }

    pub(crate) fn last_agent_message(&self) -> Option<&str> {
        self.last_agent_message.as_deref()
    }

    /// `Some` once the response completed.
    pub(crate) fn completion(&self) -> Option<&Completion> {
        self.completion.as_ref()
    }

    pub(crate) fn on_event(&mut self, event: ResponseEvent, now_ms: i64) -> Vec<Effect> {
        match event {
            ResponseEvent::Created { .. } | ResponseEvent::ToolCallInputDelta { .. } => Vec::new(),
            ResponseEvent::OutputItemAdded(item) => {
                let Some(turn_item) = parse_turn_item(&item) else {
                    return Vec::new();
                };
                let started = self.item_started(&turn_item, now_ms);
                self.active_item = Some(turn_item);
                vec![started]
            }
            ResponseEvent::OutputItemDone(item) => self.on_item_done(item, now_ms),
            ResponseEvent::OutputTextDelta(delta) => match self.active_item.as_ref() {
                Some(TurnItem::AgentMessage(message)) => {
                    vec![Effect::Emit(EventMsg::AgentMessageContentDelta(
                        AgentMessageContentDeltaEvent {
                            thread_id: self.thread_id.to_string(),
                            turn_id: self.turn_id.clone(),
                            item_id: message.id.clone(),
                            delta,
                        },
                    ))]
                }
                _ => {
                    warn!("dropping output text delta without an active agent message");
                    Vec::new()
                }
            },
            ResponseEvent::ReasoningSummaryDelta {
                delta,
                summary_index,
            } => match self.active_reasoning_id() {
                Some(item_id) => vec![Effect::Emit(EventMsg::ReasoningContentDelta(
                    ReasoningContentDeltaEvent {
                        thread_id: self.thread_id.to_string(),
                        turn_id: self.turn_id.clone(),
                        item_id,
                        delta,
                        summary_index,
                    },
                ))],
                None => {
                    warn!("dropping reasoning summary delta without an active reasoning item");
                    Vec::new()
                }
            },
            ResponseEvent::ReasoningContentDelta {
                delta,
                content_index,
            } => match self.active_reasoning_id() {
                Some(item_id) => vec![Effect::Emit(EventMsg::ReasoningRawContentDelta(
                    ReasoningRawContentDeltaEvent {
                        thread_id: self.thread_id.to_string(),
                        turn_id: self.turn_id.clone(),
                        item_id,
                        delta,
                        content_index,
                    },
                ))],
                None => {
                    warn!("dropping reasoning content delta without an active reasoning item");
                    Vec::new()
                }
            },
            ResponseEvent::ReasoningSummaryPartAdded { summary_index } => {
                match self.active_reasoning_id() {
                    Some(item_id) => vec![Effect::Emit(EventMsg::AgentReasoningSectionBreak(
                        AgentReasoningSectionBreakEvent {
                            item_id,
                            summary_index,
                        },
                    ))],
                    None => Vec::new(),
                }
            }
            // Only used for sequential summary delivery, which ether does not request.
            ResponseEvent::ReasoningSummaryDone { .. } => Vec::new(),
            ResponseEvent::Completed {
                token_usage,
                end_turn,
                ..
            } => {
                if end_turn == Some(false) {
                    self.needs_follow_up = true;
                }
                self.completion = Some(Completion { token_usage });
                Vec::new()
            }
        }
    }

    fn on_item_done(&mut self, item: ResponseItem, now_ms: i64) -> Vec<Effect> {
        let previously_active = self.active_item.take();
        if is_tool_call(&item) {
            self.needs_follow_up = true;
            return vec![Effect::Record(item.clone()), Effect::RunTool(item)];
        }

        let mut effects = Vec::new();
        if let Some(mut turn_item) = parse_turn_item(&item) {
            // Keep the id the client saw in `ItemStarted` when the done item lacks one.
            if let Some(previous) = previously_active.as_ref() {
                set_item_id(&mut turn_item, previous.id());
            } else {
                effects.push(self.item_started(&turn_item, now_ms));
            }
            if let TurnItem::AgentMessage(message) = &turn_item {
                let text = agent_message_text(&message.content);
                if !text.is_empty() {
                    self.last_agent_message = Some(text);
                }
            }
            effects.push(Effect::Emit(EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: self.thread_id,
                turn_id: self.turn_id.clone(),
                item: turn_item,
                started_at_ms: None,
                completed_at_ms: now_ms,
            })));
        }
        effects.push(Effect::Record(item));
        effects
    }

    fn item_started(&self, item: &TurnItem, now_ms: i64) -> Effect {
        Effect::Emit(EventMsg::ItemStarted(ItemStartedEvent {
            thread_id: self.thread_id,
            turn_id: self.turn_id.clone(),
            item: item.clone(),
            started_at_ms: now_ms,
        }))
    }

    fn active_reasoning_id(&self) -> Option<String> {
        match self.active_item.as_ref() {
            Some(TurnItem::Reasoning(reasoning)) => Some(reasoning.id.clone()),
            _ => None,
        }
    }
}

pub(crate) fn is_tool_call(item: &ResponseItem) -> bool {
    matches!(
        item,
        ResponseItem::FunctionCall { .. }
            | ResponseItem::CustomToolCall { .. }
            | ResponseItem::LocalShellCall { .. }
    )
}

fn agent_message_text(content: &[AgentMessageContent]) -> String {
    content
        .iter()
        .map(|part| match part {
            AgentMessageContent::Text { text } => text.as_str(),
        })
        .collect()
}

fn set_item_id(item: &mut TurnItem, id: String) {
    match item {
        TurnItem::AgentMessage(message) if message.id != id => message.id = id,
        TurnItem::Reasoning(reasoning) if reasoning.id.is_empty() => reasoning.id = id,
        _ => {}
    }
}
