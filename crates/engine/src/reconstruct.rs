// Modified from OpenAI Codex (codex-rs/core/src/session/rollout_reconstruction.rs @ 806d973): forward replay of response items from the newest compaction that carries replacement history, plus the last token count; rollback, retained/guardian context, world state, windows and turn settings are removed.
//! Rebuilds a session's state from its rollout so a resumed session continues where it left off.

use crate::history::History;
use codex_protocol::ThreadId;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ThreadHistoryMode;
use codex_rollout::RolloutItem;

pub(crate) struct RolloutReconstruction {
    pub(crate) thread_id: ThreadId,
    pub(crate) history_mode: ThreadHistoryMode,
    pub(crate) history: History,
    /// Highest numeric turn id seen, so new turns get fresh ids.
    pub(crate) last_turn_number: u64,
}

/// Replays `items` (a whole rollout, first `SessionMeta` first). Returns `None` when the rollout
/// has no `SessionMeta`.
pub(crate) fn reconstruct_rollout(items: &[RolloutItem]) -> Option<RolloutReconstruction> {
    let meta = items.iter().find_map(|item| match item {
        RolloutItem::SessionMeta(line) => Some(&line.meta),
        _ => None,
    })?;

    // Only the newest compaction can bound replay, and only when it carries the history that
    // replaced what came before it.
    let checkpoint = items
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, item)| match item {
            RolloutItem::Compacted(compacted) => Some((index, compacted)),
            _ => None,
        })
        .and_then(|(index, compacted)| {
            compacted
                .replacement_history
                .clone()
                .map(|replacement| (index, replacement))
        });

    let mut history = History::default();
    let suffix = match checkpoint {
        Some((index, replacement)) => {
            history.restore(replacement);
            &items[index + 1..]
        }
        None => items,
    };
    for item in suffix {
        match item {
            RolloutItem::ResponseItem(envelope) => history.replay(envelope),
            RolloutItem::Compacted(_) => {
                // Without replacement history the summary cannot be rebuilt here; keep the
                // items replayed so far.
                tracing::warn!("ignoring a compaction without replacement history on resume");
            }
            _ => {}
        }
    }

    let token_info = items.iter().rev().find_map(|item| match item {
        RolloutItem::EventMsg(EventMsg::TokenCount(count)) => count.info.clone(),
        _ => None,
    });
    history.set_token_info(token_info);

    let last_turn_number = items
        .iter()
        .filter_map(|item| match item {
            RolloutItem::EventMsg(EventMsg::TurnStarted(started)) => {
                started.turn_id.parse::<u64>().ok()
            }
            _ => None,
        })
        .max()
        .unwrap_or(0);

    Some(RolloutReconstruction {
        thread_id: meta.id,
        history_mode: meta.history_mode,
        history,
        last_turn_number,
    })
}
