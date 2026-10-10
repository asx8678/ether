// New in ether: the turn loop, shaped like Codex's `run_turn` and `try_run_sampling_request` (codex-rs/core/src/session/turn.rs @ 806d973) and its stream retries (core/src/responses_retry.rs), without guardian, hooks, plugins, plan mode, analytics or compaction.
//! One turn: record the user's input, then sample the model until it stops calling tools.
//! Interrupting a turn cancels the model stream and running tools; every tool call left open
//! gets an "aborted" output so history stays well formed.

use crate::context::TurnAborted;
use crate::event_mapping::parse_turn_item;
use crate::sampling::Effect;
use crate::sampling::SamplingState;
use crate::session::Session;
use codex_client::backoff;
use codex_context_fragments::ContextualUserFragment;
use codex_protocol::config_types::ModeKind;
use codex_protocol::config_types::ReasoningSummary;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::ErrorEvent;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ItemCompletedEvent;
use codex_protocol::protocol::ItemStartedEvent;
use codex_protocol::protocol::StreamErrorEvent;
use codex_protocol::protocol::TurnAbortReason;
use codex_protocol::protocol::TurnAbortedEvent;
use codex_protocol::protocol::TurnCompleteEvent;
use codex_protocol::protocol::TurnStartedEvent;
use ether_llm::ApiError;
use ether_llm::Prompt;
use ether_llm::Reasoning;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

enum TurnOutcome {
    Completed { last_agent_message: Option<String> },
    Aborted,
    Failed(ErrorEvent),
}

enum SamplingError {
    Aborted,
    Api(ApiError),
}

struct SamplingResult {
    needs_follow_up: bool,
    last_agent_message: Option<String>,
}

/// Runs one turn to completion. Emits `TurnStarted` first and exactly one of `TurnComplete`
/// or `TurnAborted` last.
pub(crate) async fn run_turn(
    sess: Arc<Session>,
    turn_id: String,
    input: Vec<ResponseItem>,
    cancel: CancellationToken,
) {
    let started_at = chrono::Utc::now();
    sess.send_event(
        &turn_id,
        EventMsg::TurnStarted(TurnStartedEvent {
            turn_attribution: None,
            turn_id: turn_id.clone(),
            root_turn_id: Some(turn_id.clone()),
            trace_id: None,
            started_at: Some(started_at.timestamp()),
            model_context_window: Some(sess.config.model.context_window),
            collaboration_mode_kind: ModeKind::Default,
        }),
    )
    .await;
    record_user_input(&sess, &turn_id, input).await;

    let outcome = loop {
        let outcome = turn_loop(&sess, &turn_id, &cancel).await;
        // Input that arrived after the last model request keeps this turn going; otherwise the
        // turn ends here, before its final event, so the client can start the next one at once.
        if matches!(outcome, TurnOutcome::Completed { .. }) {
            let pending = sess.end_turn_unless_pending(&turn_id).await;
            if !pending.is_empty() {
                record_user_input(&sess, &turn_id, pending).await;
                continue;
            }
        } else {
            sess.end_turn(&turn_id).await;
        }
        break outcome;
    };

    let completed_at = chrono::Utc::now();
    let duration_ms = Some((completed_at - started_at).num_milliseconds());
    match outcome {
        TurnOutcome::Completed { last_agent_message } => {
            sess.send_event(
                &turn_id,
                EventMsg::TurnComplete(TurnCompleteEvent {
                    root_turn_id: Some(turn_id.clone()),
                    turn_id: turn_id.clone(),
                    last_agent_message,
                    error: None,
                    started_at: Some(started_at.timestamp()),
                    completed_at: Some(completed_at.timestamp()),
                    duration_ms,
                    time_to_first_token_ms: None,
                }),
            )
            .await;
        }
        TurnOutcome::Failed(error) => {
            sess.send_event(&turn_id, EventMsg::Error(error.clone()))
                .await;
            sess.send_event(
                &turn_id,
                EventMsg::TurnComplete(TurnCompleteEvent {
                    root_turn_id: Some(turn_id.clone()),
                    turn_id: turn_id.clone(),
                    last_agent_message: None,
                    error: Some(error),
                    started_at: Some(started_at.timestamp()),
                    completed_at: Some(completed_at.timestamp()),
                    duration_ms,
                    time_to_first_token_ms: None,
                }),
            )
            .await;
        }
        TurnOutcome::Aborted => {
            sess.close_open_calls().await;
            let marker = ContextualUserFragment::into(TurnAborted::interrupted());
            sess.record_items(std::slice::from_ref(&marker)).await;
            sess.send_event(
                &turn_id,
                EventMsg::TurnAborted(TurnAbortedEvent {
                    root_turn_id: Some(turn_id.clone()),
                    turn_id: Some(turn_id.clone()),
                    reason: TurnAbortReason::Interrupted,
                    error: None,
                    started_at: Some(started_at.timestamp()),
                    completed_at: Some(completed_at.timestamp()),
                    duration_ms,
                }),
            )
            .await;
        }
    }
}

async fn record_user_input(sess: &Session, turn_id: &str, input: Vec<ResponseItem>) {
    sess.record_items(&input).await;
    for item in &input {
        let Some(turn_item) = parse_turn_item(item) else {
            continue;
        };
        let now_ms = chrono::Utc::now().timestamp_millis();
        sess.send_event(
            turn_id,
            EventMsg::ItemStarted(ItemStartedEvent {
                thread_id: sess.thread_id,
                turn_id: turn_id.to_string(),
                item: turn_item.clone(),
                started_at_ms: now_ms,
            }),
        )
        .await;
        sess.send_event(
            turn_id,
            EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: sess.thread_id,
                turn_id: turn_id.to_string(),
                item: turn_item,
                started_at_ms: Some(now_ms),
                completed_at_ms: now_ms,
            }),
        )
        .await;
    }
}

async fn turn_loop(sess: &Arc<Session>, turn_id: &str, cancel: &CancellationToken) -> TurnOutcome {
    let mut last_agent_message = None;
    loop {
        let pending = sess.take_pending_input().await;
        if !pending.is_empty() {
            record_user_input(sess, turn_id, pending).await;
        }

        match sample_with_retries(sess, turn_id, cancel).await {
            Ok(result) => {
                if result.last_agent_message.is_some() {
                    last_agent_message = result.last_agent_message;
                }
                if !result.needs_follow_up && !sess.has_pending_input().await {
                    return TurnOutcome::Completed { last_agent_message };
                }
            }
            Err(SamplingError::Aborted) => return TurnOutcome::Aborted,
            Err(SamplingError::Api(error)) => {
                return TurnOutcome::Failed(ErrorEvent {
                    message: error.to_string(),
                    codex_error_info: None,
                    misalignment: None,
                });
            }
        }
    }
}

/// Samples once, retrying stream failures up to the provider's `stream_max_retries`.
async fn sample_with_retries(
    sess: &Arc<Session>,
    turn_id: &str,
    cancel: &CancellationToken,
) -> Result<SamplingResult, SamplingError> {
    let max_retries = sess.config.model_provider.stream_max_retries();
    let mut retries = 0;
    loop {
        match run_sampling_request(sess, turn_id, cancel).await {
            Err(SamplingError::Api(error)) if retries < max_retries && is_retryable(&error) => {
                retries += 1;
                let delay = backoff(Duration::from_millis(200), retries);
                sess.send_event(
                    turn_id,
                    EventMsg::StreamError(StreamErrorEvent {
                        message: format!("Reconnecting... {retries}/{max_retries}"),
                        codex_error_info: None,
                        additional_details: Some(error.to_string()),
                    }),
                )
                .await;
                tokio::select! {
                    _ = cancel.cancelled() => return Err(SamplingError::Aborted),
                    _ = tokio::time::sleep(delay) => {}
                }
            }
            result => return result,
        }
    }
}

fn is_retryable(error: &ApiError) -> bool {
    matches!(
        error,
        ApiError::Stream(_)
            | ApiError::Transport(_)
            | ApiError::Retryable { .. }
            | ApiError::ServerOverloaded { .. }
    )
}

async fn build_prompt(sess: &Session) -> Prompt {
    let config = &sess.config;
    let reasoning = config.model.supports_reasoning.then(|| Reasoning {
        effort: config.reasoning_effort.clone(),
        summary: (config.reasoning_summary != ReasoningSummary::None)
            .then_some(config.reasoning_summary),
    });
    Prompt {
        model: config.model.slug.clone(),
        instructions: config.base_instructions.clone(),
        input: sess.prompt_input().await,
        tools: sess.tools.specs(),
        parallel_tool_calls: config.model.supports_parallel_tool_calls,
        reasoning,
        verbosity: None,
        output_schema: None,
    }
}

/// One model request: stream the response, apply each event's effects, then wait for the
/// tools it started and record their outputs in call order.
async fn run_sampling_request(
    sess: &Arc<Session>,
    turn_id: &str,
    cancel: &CancellationToken,
) -> Result<SamplingResult, SamplingError> {
    let prompt = build_prompt(sess).await;
    let mut stream = tokio::select! {
        _ = cancel.cancelled() => return Err(SamplingError::Aborted),
        stream = sess.model.stream(&prompt) => stream.map_err(SamplingError::Api)?,
    };

    let mut state = SamplingState::new(sess.thread_id, turn_id.to_string());
    let tools_cancel = cancel.child_token();
    let mut in_flight: Vec<JoinHandle<ResponseItem>> = Vec::new();
    let stream_result = loop {
        let event = tokio::select! {
            _ = cancel.cancelled() => break Err(SamplingError::Aborted),
            event = stream.rx_event.recv() => event,
        };
        let event = match event {
            Some(Ok(event)) => event,
            Some(Err(error)) => break Err(SamplingError::Api(error)),
            None => {
                break Err(SamplingError::Api(ApiError::Stream(
                    "stream closed before response.completed".to_string(),
                )));
            }
        };
        let now_ms = chrono::Utc::now().timestamp_millis();
        for effect in state.on_event(event, now_ms) {
            match effect {
                Effect::Emit(msg) => sess.send_event(turn_id, msg).await,
                Effect::Record(item) => sess.record_items(std::slice::from_ref(&item)).await,
                Effect::RunTool(call) => {
                    let sess = Arc::clone(sess);
                    let cancel = tools_cancel.clone();
                    in_flight.push(tokio::spawn(async move {
                        tokio::select! {
                            output = sess.tools.dispatch(&call) => output,
                            _ = cancel.cancelled() => crate::tools::output_for(
                                &call,
                                call_id_of(&call),
                                "aborted".to_string(),
                            ),
                        }
                    }));
                }
            }
        }
        if state.completion().is_some() {
            break Ok(());
        }
    };

    if let Err(error) = stream_result {
        tools_cancel.cancel();
        for handle in in_flight {
            handle.abort();
        }
        return Err(error);
    }

    for handle in in_flight {
        let output = tokio::select! {
            _ = cancel.cancelled() => {
                tools_cancel.cancel();
                return Err(SamplingError::Aborted);
            }
            output = handle => output,
        };
        match output {
            Ok(output) => sess.record_items(std::slice::from_ref(&output)).await,
            Err(join_error) => tracing::error!("tool task failed: {join_error}"),
        }
    }

    if let Some(usage) = state
        .completion()
        .and_then(|completion| completion.token_usage.as_ref())
    {
        let token_count = sess.update_token_usage(usage).await;
        sess.send_event(turn_id, EventMsg::TokenCount(token_count))
            .await;
    }

    Ok(SamplingResult {
        needs_follow_up: state.needs_follow_up(),
        last_agent_message: state.last_agent_message().map(str::to_string),
    })
}

fn call_id_of(call: &ResponseItem) -> String {
    match call {
        ResponseItem::FunctionCall { call_id, .. }
        | ResponseItem::CustomToolCall { call_id, .. } => call_id.clone(),
        ResponseItem::LocalShellCall { call_id, .. } => call_id.clone().unwrap_or_default(),
        _ => String::new(),
    }
}
