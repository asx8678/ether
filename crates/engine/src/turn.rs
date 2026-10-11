// New in ether: the turn loop, shaped like Codex's `run_turn` and `try_run_sampling_request` (codex-rs/core/src/session/turn.rs @ 806d973) and its stream retries (core/src/responses_retry.rs), without guardian, hooks, plugins, plan mode, analytics or remote compaction; records world-state diffs at turn start and compacts locally when the context window fills.
//! One turn: record the user's input, then sample the model until it stops calling tools.
//! Interrupting a turn cancels the model stream and running tools; every tool call left open
//! gets an "aborted" output so history stays well formed.

use crate::compact::CompactError;
use crate::compact::run_compact;
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
use codex_protocol::protocol::TurnDiffEvent;
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
    let started_at = emit_turn_started(&sess, &turn_id).await;
    sess.record_turn_context(&turn_id).await;
    // World-state items the model has not seen go before the user's message.
    sess.record_world_state().await;
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
        }
        break outcome;
    };
    if matches!(outcome, TurnOutcome::Completed { .. }) {
        emit_turn_end(&sess, &turn_id, started_at, outcome).await;
        return;
    }
    // An aborted or failed turn is cleaned up and reported while it still holds the session,
    // so no other turn interleaves; input sent meanwhile starts the next turn.
    abandon_turn(&sess, matches!(outcome, TurnOutcome::Aborted)).await;
    emit_turn_end(&sess, &turn_id, started_at, outcome).await;
    let pending = sess.end_turn_take_pending(&turn_id).await;
    if !pending.is_empty() {
        spawn_turn(&sess, pending).await;
    }
}

/// Answers the calls the turn leaves open and drops its pending requests; an interrupted
/// turn also records the marker that tells the model it was stopped.
async fn abandon_turn(sess: &Session, interrupted: bool) {
    sess.clear_pending_requests().await;
    sess.close_open_calls().await;
    if interrupted {
        let marker = ContextualUserFragment::into(TurnAborted::interrupted());
        sess.record_items(std::slice::from_ref(&marker)).await;
    }
}

/// Starts a turn with `items` and remembers its task; returns the turn id, or `None` when a
/// turn is already running or the session is shutting down.
pub(crate) async fn spawn_turn(sess: &Arc<Session>, items: Vec<ResponseItem>) -> Option<String> {
    let (turn_id, cancel) = sess.begin_turn().await?;
    let task = tokio::spawn(boxed_turn(Arc::clone(sess), turn_id.clone(), items, cancel));
    sess.set_turn_task(task).await;
    Some(turn_id)
}

/// `run_turn` behind a `Send` box: a turn can start the next one, and the named type breaks
/// the otherwise recursive future type.
fn boxed_turn(
    sess: Arc<Session>,
    turn_id: String,
    items: Vec<ResponseItem>,
    cancel: CancellationToken,
) -> futures::future::BoxFuture<'static, ()> {
    Box::pin(run_turn(sess, turn_id, items, cancel))
}

/// Starts a compaction turn and remembers its task; `false` when a turn is running.
pub(crate) async fn spawn_compact_turn(sess: &Arc<Session>) -> bool {
    let Some((turn_id, cancel)) = sess.begin_turn().await else {
        return false;
    };
    let task = tokio::spawn(boxed_compact_turn(Arc::clone(sess), turn_id, cancel));
    sess.set_turn_task(task).await;
    true
}

fn boxed_compact_turn(
    sess: Arc<Session>,
    turn_id: String,
    cancel: CancellationToken,
) -> futures::future::BoxFuture<'static, ()> {
    Box::pin(run_compact_turn(sess, turn_id, cancel))
}

/// Runs a turn that only compacts the conversation (`Op::Compact`). Input sent while it runs
/// starts a normal turn afterwards.
async fn run_compact_turn(sess: Arc<Session>, turn_id: String, cancel: CancellationToken) {
    let started_at = emit_turn_started(&sess, &turn_id).await;
    let outcome = match run_compact(&sess, &turn_id, &cancel).await {
        Ok(()) => TurnOutcome::Completed {
            last_agent_message: None,
        },
        Err(error) => compact_error_outcome(error),
    };
    if !matches!(outcome, TurnOutcome::Completed { .. }) {
        sess.clear_pending_requests().await;
    }
    emit_turn_end(&sess, &turn_id, started_at, outcome).await;
    let pending = sess.end_turn_take_pending(&turn_id).await;
    if !pending.is_empty() {
        spawn_turn(&sess, pending).await;
    }
}

fn compact_error_outcome(error: CompactError) -> TurnOutcome {
    match error {
        CompactError::Aborted => TurnOutcome::Aborted,
        CompactError::Api(error) => TurnOutcome::Failed(ErrorEvent {
            message: format!("Error running local compact task: {error}"),
            codex_error_info: None,
            misalignment: None,
        }),
    }
}

async fn emit_turn_started(sess: &Session, turn_id: &str) -> chrono::DateTime<chrono::Utc> {
    let started_at = chrono::Utc::now();
    sess.send_event(
        turn_id,
        EventMsg::TurnStarted(TurnStartedEvent {
            turn_attribution: None,
            turn_id: turn_id.to_string(),
            root_turn_id: Some(turn_id.to_string()),
            trace_id: None,
            started_at: Some(started_at.timestamp()),
            model_context_window: Some(sess.config.model.context_window),
            collaboration_mode_kind: ModeKind::Default,
        }),
    )
    .await;
    started_at
}

/// Emits a turn's final event.
async fn emit_turn_end(
    sess: &Session,
    turn_id: &str,
    started_at: chrono::DateTime<chrono::Utc>,
    outcome: TurnOutcome,
) {
    let turn_id = turn_id.to_string();
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
    let mut compacted_for_overflow = false;
    loop {
        let pending = sess.take_pending_input().await;
        if !pending.is_empty() {
            record_user_input(sess, turn_id, pending).await;
        }

        // Compact when the last request filled the usable window: before the first request of a
        // turn, or mid-turn when the model still has work to do.
        if sess.context_window_exhausted().await
            && let Err(error) = run_compact(sess, turn_id, cancel).await
        {
            return compact_error_outcome(error);
        }

        match sample_with_retries(sess, turn_id, cancel).await {
            // The provider says the request does not fit: compact once and try again.
            Err(SamplingError::Api(ApiError::ContextWindowExceeded)) if !compacted_for_overflow => {
                compacted_for_overflow = true;
                if let Err(error) = run_compact(sess, turn_id, cancel).await {
                    return compact_error_outcome(error);
                }
            }
            Ok(result) => {
                compacted_for_overflow = false;
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

/// Samples once, retrying stream failures up to the provider's `stream_max_retries`. A rate
/// limit whose `Retry-After` is longer than [`MAX_RETRY_AFTER`] parks the turn until the reset
/// (up to [`MAX_PARK`], at most [`MAX_PARKS`] times) without using up a retry.
async fn sample_with_retries(
    sess: &Arc<Session>,
    turn_id: &str,
    cancel: &CancellationToken,
) -> Result<SamplingResult, SamplingError> {
    let max_retries = sess.config.model_provider.stream_max_retries();
    let mut retries = 0;
    let mut parks = 0;
    loop {
        match run_sampling_request(sess, turn_id, cancel).await {
            Err(SamplingError::Api(error)) if is_retryable(&error) => {
                let server_delay = match &error {
                    ApiError::RateLimitExceeded {
                        retry_after: Some(after),
                        ..
                    }
                    | ApiError::Retryable {
                        retry_after: Some(after),
                        ..
                    } => Some(after.remaining_delay()),
                    _ => None,
                };
                let (delay, message) = match server_delay {
                    Some(wait) if wait > MAX_RETRY_AFTER => {
                        if parks >= MAX_PARKS || wait > MAX_PARK {
                            return Err(SamplingError::Api(error));
                        }
                        parks += 1;
                        // Jitter, but never past the longest park.
                        let wait = with_jitter(wait).min(MAX_PARK);
                        let until = chrono::Local::now()
                            + chrono::Duration::from_std(wait).unwrap_or_default();
                        (
                            wait,
                            format!("Rate limited; waiting until {}", until.format("%H:%M:%S")),
                        )
                    }
                    _ if retries >= max_retries => return Err(SamplingError::Api(error)),
                    // A rate limit's short `Retry-After` wins over the exponential backoff
                    // (which carries its own jitter).
                    Some(wait) => {
                        retries += 1;
                        (
                            with_jitter(wait),
                            format!("Reconnecting... {retries}/{max_retries}"),
                        )
                    }
                    None => {
                        retries += 1;
                        (
                            backoff(Duration::from_millis(200), retries),
                            format!("Reconnecting... {retries}/{max_retries}"),
                        )
                    }
                };
                sess.send_event(
                    turn_id,
                    EventMsg::StreamError(StreamErrorEvent {
                        message,
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

/// The longest `Retry-After` the engine treats as a quick retry; longer ones park the turn.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

/// The longest reset a turn waits for (quota windows); a longer one fails the turn.
const MAX_PARK: Duration = Duration::from_secs(15 * 60);

/// How many times one sampling request may park.
const MAX_PARKS: u32 = 3;

/// Waits a little past a provider-reported reset, never before it: up to 20% longer, so
/// sessions limited together do not all retry at the same instant.
fn with_jitter(wait: Duration) -> Duration {
    wait.mul_f64(1.0 + rand::random::<f64>() * 0.2)
}

fn is_retryable(error: &ApiError) -> bool {
    matches!(
        error,
        ApiError::Stream(_)
            | ApiError::Transport(_)
            | ApiError::Retryable { .. }
            | ApiError::RateLimitExceeded { .. }
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
        glm_thinking: config.model.glm_thinking,
        cache_key: Some(sess.thread_id.to_string()),
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
                    let turn_id = turn_id.to_string();
                    let cancel = tools_cancel.clone();
                    in_flight.push(tokio::spawn(async move {
                        let tools = &sess.tools;
                        tokio::select! {
                            output = tools.dispatch(Arc::clone(&sess), turn_id, &call, cancel.clone()) => output,
                            _ = cancel.cancelled() => crate::tools::output_for(
                                &call,
                                crate::tools::call_id_of(&call),
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

    // Report the turn's diff after a request whose tools changed files.
    let turn_diff = sess.turn_diffs.lock().await.take_changed(turn_id);
    if let Some(unified_diff) = turn_diff {
        sess.send_event(turn_id, EventMsg::TurnDiff(TurnDiffEvent { unified_diff }))
            .await;
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
