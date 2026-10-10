// Modified from OpenAI Codex (codex-rs/core/src/context_manager/history.rs @ 806d973): recording, rollout replay, prompt normalization and token accounting only; retained context, guardian review history, rollback and media stripping are removed.
//! The conversation as the model sees it. History is append-only: items are recorded once, tool
//! output is truncated when it is recorded, and prompts are built from a normalized copy, so each
//! request's input starts with the previous request's input.

use crate::normalize;
use codex_history::ResponseItemEnvelope;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use codex_protocol::protocol::TokenUsageInfo;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::truncate_function_output_payload;
use codex_utils_output_truncation::with_serialization_allowance;
use std::collections::HashSet;

#[derive(Debug, Clone, Default)]
pub(crate) struct History {
    items: Vec<ResponseItemEnvelope>,
    token_info: Option<TokenUsageInfo>,
}

impl History {
    /// Appends model-visible items, truncating tool output to `policy`. Returns the entries
    /// added, as they were stored, so the rollout records exactly what history holds.
    pub(crate) fn record(
        &mut self,
        items: &[ResponseItem],
        policy: TruncationPolicy,
    ) -> Vec<ResponseItemEnvelope> {
        let mut added = Vec::new();
        for item in items {
            if !is_api_message(item) {
                continue;
            }
            let mut item = item.clone();
            if let ResponseItem::FunctionCallOutput { output, .. }
            | ResponseItem::CustomToolCallOutput { output, .. } = &mut item
            {
                truncate_function_output_payload(
                    output,
                    with_serialization_allowance(policy),
                    estimate_audio_token_count,
                );
            }
            let envelope = ResponseItemEnvelope::new(item);
            added.push(envelope.clone());
            self.items.push(envelope);
        }
        added
    }

    /// Appends entries read back from a rollout. They were truncated when first recorded.
    pub(crate) fn replay(&mut self, envelope: &ResponseItemEnvelope) {
        if is_api_message(&envelope.item) {
            self.items.push(envelope.clone());
        }
    }

    /// Replaces everything with a compaction's replacement history.
    pub(crate) fn replace(&mut self, items: Vec<ResponseItemEnvelope>) {
        self.items = items;
    }

    pub(crate) fn set_token_info(&mut self, token_info: Option<TokenUsageInfo>) {
        self.token_info = token_info;
    }

    /// Returns the normalized history to send as the next request's input.
    pub(crate) fn for_prompt(&self) -> Vec<ResponseItem> {
        let mut items = self.items.clone();
        normalize::ensure_call_outputs_present(&mut items);
        normalize::remove_orphan_outputs(&mut items);
        items
            .into_iter()
            .map(ResponseItemEnvelope::into_item)
            .collect()
    }

    /// Gives every recorded tool call an output, so an interrupted turn leaves no call
    /// without a result in history (and in the rollout written from it). Returns the
    /// outputs it added.
    pub(crate) fn close_open_calls(&mut self) -> Vec<ResponseItem> {
        let answered: HashSet<String> = self
            .items
            .iter()
            .filter_map(|envelope| output_call_id(&envelope.item))
            .map(str::to_string)
            .collect();
        normalize::ensure_call_outputs_present(&mut self.items);
        self.items
            .iter()
            .filter(|envelope| {
                output_call_id(&envelope.item).is_some_and(|call_id| !answered.contains(call_id))
            })
            .map(|envelope| envelope.item.clone())
            .collect()
    }

    pub(crate) fn token_info(&self) -> Option<&TokenUsageInfo> {
        self.token_info.as_ref()
    }

    pub(crate) fn update_token_info(&mut self, usage: &TokenUsage, model_context_window: i64) {
        self.token_info = TokenUsageInfo::new_or_append(
            &self.token_info,
            &Some(usage.clone()),
            Some(model_context_window),
        );
    }
}

fn is_api_message(message: &ResponseItem) -> bool {
    match message {
        ResponseItem::Message { role, .. } => role.as_str() != "system",
        ResponseItem::AdditionalTools { .. }
        | ResponseItem::AgentMessage { .. }
        | ResponseItem::FunctionCallOutput { .. }
        | ResponseItem::FunctionCall { .. }
        | ResponseItem::ToolSearchCall { .. }
        | ResponseItem::ToolSearchOutput { .. }
        | ResponseItem::CustomToolCall { .. }
        | ResponseItem::CustomToolCallOutput { .. }
        | ResponseItem::LocalShellCall { .. }
        | ResponseItem::Reasoning { .. }
        | ResponseItem::WebSearchCall { .. }
        | ResponseItem::ImageGenerationCall { .. }
        | ResponseItem::Compaction { .. }
        | ResponseItem::ContextCompaction { .. } => true,
        ResponseItem::ConfigurationUpdate { .. }
        | ResponseItem::CompactionTrigger { .. }
        | ResponseItem::Other => false,
    }
}

fn output_call_id(item: &ResponseItem) -> Option<&str> {
    match item {
        ResponseItem::FunctionCallOutput {
            call_id: Some(call_id),
            ..
        }
        | ResponseItem::ToolSearchOutput {
            call_id: Some(call_id),
            ..
        }
        | ResponseItem::CustomToolCallOutput { call_id, .. } => Some(call_id),
        _ => None,
    }
}

fn estimate_audio_token_count(_audio: &str) -> usize {
    0
}
