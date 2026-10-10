// Modified from OpenAI Codex (codex-rs/core/src/event_mapping.rs @ 806d973): user, assistant and reasoning items only; hook prompts, web search and image generation removed; contextual-message detection uses the engine's own marker list and skips compaction summaries.
//! Maps model-visible history items to the turn items clients render.

use codex_protocol::items::AgentMessageContent;
use codex_protocol::items::AgentMessageItem;
use codex_protocol::items::ReasoningItem;
use codex_protocol::items::TurnItem;
use codex_protocol::items::UserMessageItem;
use codex_protocol::models::ContentItem;
use codex_protocol::models::MessagePhase;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ReasoningItemReasoningSummary;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::is_image_close_tag_text;
use codex_protocol::models::is_image_open_tag_text;
use codex_protocol::models::is_local_image_close_tag_text;
use codex_protocol::models::is_local_image_open_tag_text;
use codex_protocol::user_input::UserInput;
use tracing::warn;
use uuid::Uuid;

/// Opening markers of context the engine injects as user messages. Such messages are model
/// context, not something the user typed, so they never become `UserMessage` items.
pub(crate) const CONTEXTUAL_USER_MARKERS: &[&str] = &[
    "<environment_context>",
    "<user_instructions>",
    "# AGENTS.md instructions",
    "<turn_aborted>",
];

fn is_contextual_user_text(text: &str) -> bool {
    if crate::compact::is_summary_message(text) {
        return true;
    }
    let trimmed = text.trim_start();
    CONTEXTUAL_USER_MARKERS.iter().any(|marker| {
        trimmed
            .get(..marker.len())
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(marker))
    })
}

pub(crate) fn is_contextual_user_message_content(message: &[ContentItem]) -> bool {
    message.iter().any(|item| match item {
        ContentItem::InputText { text } => is_contextual_user_text(text),
        _ => false,
    })
}

fn parse_user_message(message: &[ContentItem]) -> Option<UserMessageItem> {
    if is_contextual_user_message_content(message) {
        return None;
    }

    let mut content: Vec<UserInput> = Vec::new();

    for (idx, content_item) in message.iter().enumerate() {
        match content_item {
            ContentItem::InputText { text } => {
                let is_image_label = ((is_local_image_open_tag_text(text)
                    || is_image_open_tag_text(text))
                    && matches!(message.get(idx + 1), Some(ContentItem::InputImage { .. })))
                    || (idx > 0
                        && (is_local_image_close_tag_text(text) || is_image_close_tag_text(text))
                        && matches!(message.get(idx - 1), Some(ContentItem::InputImage { .. })));
                if is_image_label {
                    continue;
                }
                content.push(UserInput::Text {
                    text: text.clone(),
                    // Model input content does not carry UI element ranges.
                    text_elements: Vec::new(),
                });
            }
            ContentItem::InputImage { image, detail } => {
                content.push(UserInput::Image {
                    image: image.clone(),
                    detail: *detail,
                });
            }
            ContentItem::InputAudio { .. } => {
                warn!("Audio input in user message is not supported");
            }
            ContentItem::OutputText { text, .. } => {
                warn!("Output text in user message: {}", text);
            }
        }
    }

    Some(UserMessageItem::new(&content))
}

fn parse_agent_message(
    id: Option<&str>,
    message: &[ContentItem],
    phase: Option<MessagePhase>,
) -> AgentMessageItem {
    let mut content: Vec<AgentMessageContent> = Vec::new();
    for content_item in message.iter() {
        match content_item {
            ContentItem::InputText { text } | ContentItem::OutputText { text, .. } => {
                content.push(AgentMessageContent::Text { text: text.clone() });
            }
            _ => {
                warn!(
                    "Unexpected content item in agent message: {:?}",
                    content_item
                );
            }
        }
    }
    let id = id
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    AgentMessageItem {
        id,
        content,
        phase,
        memory_citation: None,
        delivery: None,
        questions: None,
    }
}

pub fn parse_turn_item(item: &ResponseItem) -> Option<TurnItem> {
    match item {
        ResponseItem::Message {
            role,
            content,
            id,
            phase,
            ..
        } => match role.as_str() {
            "user" => parse_user_message(content).map(TurnItem::UserMessage),
            "assistant" => Some(TurnItem::AgentMessage(parse_agent_message(
                id.as_deref(),
                content,
                phase.clone(),
            ))),
            _ => None,
        },
        ResponseItem::Reasoning {
            id,
            summary,
            content,
            ..
        } => {
            let summary_text = summary
                .iter()
                .map(|entry| match entry {
                    ReasoningItemReasoningSummary::SummaryText { text } => text.clone(),
                })
                .collect();
            let raw_content = content
                .clone()
                .unwrap_or_default()
                .into_iter()
                .map(|entry| match entry {
                    ReasoningItemContent::ReasoningText { text }
                    | ReasoningItemContent::Text { text } => text,
                })
                .collect();
            Some(TurnItem::Reasoning(ReasoningItem {
                id: id.as_deref().unwrap_or_default().to_string(),
                summary_text,
                raw_content,
            }))
        }
        _ => None,
    }
}
