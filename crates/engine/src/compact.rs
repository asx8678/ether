// Modified from OpenAI Codex (codex-rs/core/src/compact.rs, core/src/context/compaction_summary.rs and codex-rs/prompts/src/compact.rs @ 806d973): local (Responses summarization) compaction only; no remote compaction, hooks, analytics, goals, harness metadata, post-turn phase or auto-compact windows.
//! Local compaction: ask the model to summarize the conversation, then replace history with
//! the recent user messages, fresh initial context and the summary.

use crate::event_mapping::parse_turn_item;
use crate::session::Session;
use crate::world_state::WorldState;
use codex_client::backoff;
use codex_context_fragments::ContextualUserFragment;
use codex_protocol::items::ContextCompactionItem;
use codex_protocol::items::TurnItem;
use codex_protocol::models::AgentMessageInputContent;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ContentItemKind;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::ContextCompactedEvent;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ItemCompletedEvent;
use codex_protocol::protocol::ItemStartedEvent;
use codex_protocol::protocol::StreamErrorEvent;
use codex_protocol::protocol::WarningEvent;
use codex_protocol::user_input::UserInput;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::approx_token_count;
use codex_utils_output_truncation::truncate_text;
use ether_llm::ApiError;
use ether_llm::Prompt;
use ether_llm::ResponseEvent;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub(crate) const SUMMARIZATION_PROMPT: &str = include_str!("../templates/compact/prompt.md");
pub(crate) const SUMMARY_PREFIX: &str = include_str!("../templates/compact/summary_prefix.md");
const COMPACT_USER_MESSAGE_MAX_TOKENS: usize = 20_000;

/// Why compaction did not finish.
pub(crate) enum CompactError {
    Aborted,
    Api(ApiError),
}

/// Summarizes the conversation and replaces history with the compacted form. Emits the
/// `ContextCompaction` item lifecycle, `ContextCompacted` and Codex's accuracy warning.
pub(crate) async fn run_compact(
    sess: &Session,
    turn_id: &str,
    cancel: &CancellationToken,
) -> Result<(), CompactError> {
    let compaction_item = TurnItem::ContextCompaction(ContextCompactionItem::new());
    let started_at_ms = chrono::Utc::now().timestamp_millis();
    sess.send_event(
        turn_id,
        EventMsg::ItemStarted(ItemStartedEvent {
            thread_id: sess.thread_id,
            turn_id: turn_id.to_string(),
            item: compaction_item.clone(),
            started_at_ms,
        }),
    )
    .await;

    let history = sess.prompt_input().await;
    let summary_suffix = request_summary(sess, turn_id, cancel, &history).await?;
    let summary_text = format!("{SUMMARY_PREFIX}\n{summary_suffix}");
    let user_messages = collect_user_messages(&history);
    let compacted = build_compacted_history(&user_messages, &summary_text);

    // The compacted history drops the old world-state items, so render it again in full and
    // make that the baseline for later diffs.
    let world_state = WorldState::capture(&sess.config);
    let (snapshot, initial_context) = world_state.render_diff(/*previous*/ None);
    let new_history =
        insert_initial_context_before_last_real_user_or_summary(compacted, initial_context);
    let token_count = sess.replace_compacted_history(new_history, snapshot).await;
    sess.send_event(turn_id, EventMsg::TokenCount(token_count))
        .await;

    sess.send_event(
        turn_id,
        EventMsg::ItemCompleted(ItemCompletedEvent {
            thread_id: sess.thread_id,
            turn_id: turn_id.to_string(),
            item: compaction_item,
            started_at_ms: Some(started_at_ms),
            completed_at_ms: chrono::Utc::now().timestamp_millis(),
        }),
    )
    .await;
    sess.send_event(turn_id, EventMsg::ContextCompacted(ContextCompactedEvent))
        .await;
    sess.send_event(
        turn_id,
        EventMsg::Warning(WarningEvent {
            message: "Heads up: Long threads and multiple compactions can cause the model to be less accurate. Start a new thread when possible to keep threads small and targeted.".to_string(),
        }),
    )
    .await;
    Ok(())
}

/// Runs the summarization request with Codex's retry rules: stream failures retry with
/// backoff; a context-window overflow drops the oldest item and tries again.
async fn request_summary(
    sess: &Session,
    turn_id: &str,
    cancel: &CancellationToken,
    history: &[ResponseItem],
) -> Result<String, CompactError> {
    let prompt_item = ResponseItem::from(ResponseInputItem::from(vec![UserInput::Text {
        text: SUMMARIZATION_PROMPT.to_string(),
        // Compaction prompt is synthesized; no UI element ranges to preserve.
        text_elements: Vec::new(),
    }]));
    let mut input = history.to_vec();
    let max_retries = sess.config.model_provider.stream_max_retries();
    let mut retries = 0;
    loop {
        let mut request_input = input.clone();
        request_input.push(prompt_item.clone());
        let prompt = Prompt {
            model: sess.config.model.slug.clone(),
            instructions: sess.config.base_instructions.clone(),
            input: request_input,
            tools: Vec::new(),
            parallel_tool_calls: false,
            reasoning: None,
            verbosity: None,
            output_schema: None,
        };
        match drain_to_completed(sess, cancel, &prompt).await {
            Ok(output) => return Ok(last_assistant_message(&output).unwrap_or_default()),
            Err(CompactError::Api(ApiError::ContextWindowExceeded)) if input.len() > 1 => {
                // Trim from the beginning to keep recent messages intact.
                tracing::error!(
                    "Context window exceeded while compacting; removing oldest history item."
                );
                input.remove(0);
                retries = 0;
            }
            Err(CompactError::Api(error))
                if retries < max_retries && !matches!(error, ApiError::ContextWindowExceeded) =>
            {
                retries += 1;
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
                    _ = cancel.cancelled() => return Err(CompactError::Aborted),
                    _ = tokio::time::sleep(backoff(Duration::from_millis(200), retries)) => {}
                }
            }
            Err(error) => return Err(error),
        }
    }
}

/// Streams one response and returns its output items.
async fn drain_to_completed(
    sess: &Session,
    cancel: &CancellationToken,
    prompt: &Prompt,
) -> Result<Vec<ResponseItem>, CompactError> {
    let mut stream = tokio::select! {
        _ = cancel.cancelled() => return Err(CompactError::Aborted),
        stream = sess.model.stream(prompt) => stream.map_err(CompactError::Api)?,
    };
    let mut output = Vec::new();
    loop {
        let event = tokio::select! {
            _ = cancel.cancelled() => return Err(CompactError::Aborted),
            event = stream.rx_event.recv() => event,
        };
        match event {
            Some(Ok(ResponseEvent::OutputItemDone(item))) => output.push(item),
            Some(Ok(ResponseEvent::Completed { token_usage, .. })) => {
                // Counted in the session totals, as Codex does.
                if let Some(usage) = &token_usage {
                    sess.update_token_usage(usage).await;
                }
                return Ok(output);
            }
            Some(Ok(_)) => {}
            Some(Err(error)) => return Err(CompactError::Api(error)),
            None => {
                return Err(CompactError::Api(ApiError::Stream(
                    "stream closed before response.completed".to_string(),
                )));
            }
        }
    }
}

fn last_assistant_message(items: &[ResponseItem]) -> Option<String> {
    items.iter().rev().find_map(|item| match item {
        ResponseItem::Message { role, content, .. } if role == "assistant" => {
            content_items_to_text(content)
        }
        _ => None,
    })
}

fn content_items_to_text(content: &[ContentItem]) -> Option<String> {
    let mut pieces = Vec::new();
    for item in content {
        match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text, .. } => {
                if !text.is_empty() {
                    pieces.push(text.as_str());
                }
            }
            ContentItem::InputImage { .. } | ContentItem::InputAudio { .. } => {}
        }
    }
    if pieces.is_empty() {
        None
    } else {
        Some(pieces.join("\n"))
    }
}

/// A user message kept across compaction, with its flattened text for the token budget.
struct CompactedUserMessage<'a> {
    message: String,
    original: &'a ResponseItem,
}

fn collect_user_messages(items: &[ResponseItem]) -> Vec<CompactedUserMessage<'_>> {
    items
        .iter()
        .filter_map(|item| {
            let Some(TurnItem::UserMessage(user)) = parse_turn_item(item) else {
                return None;
            };
            let message = user.message();
            (!is_summary_message(&message)).then_some(CompactedUserMessage {
                message,
                original: item,
            })
        })
        .collect()
}

pub(crate) fn is_summary_message(message: &str) -> bool {
    message.starts_with(format!("{SUMMARY_PREFIX}\n").as_str())
}

/// Keeps the most recent user messages within the token budget (truncating the oldest one
/// that crosses it), then appends the summary.
fn build_compacted_history(
    user_messages: &[CompactedUserMessage<'_>],
    summary_text: &str,
) -> Vec<ResponseItem> {
    let mut selected_messages = Vec::new();
    let mut remaining = COMPACT_USER_MESSAGE_MAX_TOKENS;
    for message in user_messages.iter().rev() {
        if remaining == 0 {
            break;
        }
        let tokens = approx_token_count(&message.message);
        let ResponseItem::Message {
            id,
            content,
            internal_chat_message_metadata_passthrough,
            ..
        } = message.original
        else {
            continue;
        };
        let mut passthrough = internal_chat_message_metadata_passthrough.clone();
        let content = if tokens <= remaining
            && content
                .iter()
                .all(|part| matches!(part, ContentItem::InputText { .. }))
        {
            content.clone()
        } else {
            // Rebuild only the text fallback; never clone discarded media.
            if let Some(kinds) = passthrough
                .as_mut()
                .and_then(|metadata| metadata.content_item_kinds.as_mut())
            {
                *kinds = vec![ContentItemKind("user.text".to_owned())];
            }
            vec![ContentItem::InputText {
                text: truncate_text(&message.message, TruncationPolicy::Tokens(remaining)),
            }]
        };
        selected_messages.push(ResponseItem::Message {
            status: None,
            encrypted_content: None,
            id: id.clone(),
            role: "user".to_owned(),
            content,
            phase: None,
            internal_chat_message_metadata_passthrough: passthrough,
        });
        if tokens > remaining {
            break;
        }
        remaining = remaining.saturating_sub(tokens);
    }
    selected_messages.reverse();

    let summary_text = if summary_text.is_empty() {
        "(no summary available)".to_string()
    } else {
        summary_text.to_string()
    };
    selected_messages.push(ContextualUserFragment::into(CompactionSummary {
        summary: summary_text,
    }));
    selected_messages
}

/// Inserts initial context before the last real user (or agent) message; failing that,
/// before the summary so it stays last; failing that, at the end.
fn insert_initial_context_before_last_real_user_or_summary(
    mut compacted_history: Vec<ResponseItem>,
    initial_context: Vec<ResponseItem>,
) -> Vec<ResponseItem> {
    let mut last_user_or_summary_index = None;
    let mut last_real_user_index = None;
    for (i, item) in compacted_history.iter().enumerate().rev() {
        if let ResponseItem::AgentMessage { content, .. } = item
            && !matches!(
                content.first(),
                Some(AgentMessageInputContent::InputText { text })
                    if text.starts_with("Message Type: FINAL_ANSWER\n")
            )
        {
            last_real_user_index = Some(i);
            break;
        }
        let Some(TurnItem::UserMessage(user)) = parse_turn_item(item) else {
            // The summary is contextual, so it is not parsed as a user message.
            if is_compaction_summary(item) {
                last_user_or_summary_index.get_or_insert(i);
            }
            continue;
        };
        last_user_or_summary_index.get_or_insert(i);
        if !is_summary_message(&user.message()) {
            last_real_user_index = Some(i);
            break;
        }
    }
    match last_real_user_index.or(last_user_or_summary_index) {
        Some(index) => {
            compacted_history.splice(index..index, initial_context);
        }
        None => compacted_history.extend(initial_context),
    }
    compacted_history
}

fn is_compaction_summary(item: &ResponseItem) -> bool {
    matches!(
        item,
        ResponseItem::Message { role, content, .. }
            if role == "user"
                && matches!(content.first(), Some(ContentItem::InputText { text }) if is_summary_message(text))
    )
}

/// The summary that replaces compacted history, as a user message.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CompactionSummary {
    summary: String,
}

impl ContextualUserFragment for CompactionSummary {
    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("compaction.summary".to_string())
    }

    fn role(&self) -> &'static str {
        "user"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("", "")
    }

    fn body(&self) -> String {
        self.summary.clone()
    }
}
