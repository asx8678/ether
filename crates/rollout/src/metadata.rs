// Modified from OpenAI Codex (codex-rs/rollout/src/metadata.rs @ 806d973): removed unit tests; removed SQLite metadata extraction and backfill (codex-state is not copied).
use crate::rollout_file_name::RolloutFileName;
use codex_protocol::RolloutId;
use codex_protocol::protocol::SessionMeta;
use std::path::Path;

/// Returns the rollout ID encoded in a canonical rollout filename.
///
/// Normal rollouts use `rollout-<timestamp>-<thread-id>.jsonl`, where the thread ID and rollout ID
/// are the same. Threads that have been `reverted` use
/// `rollout-<timestamp>-<thread-id>_<rollout-id>.jsonl`, where this returns the ID after `_`.
///
/// This can differ from [`SessionMeta::id`] when `thread/revert` keeps the thread ID stable while
/// switching to a new immutable rollout file.
pub fn rollout_id_from_path(rollout_path: &Path) -> Option<RolloutId> {
    let file_name = rollout_path.file_name()?.to_str()?;
    Some(RolloutFileName::parse(file_name)?.rollout_id())
}

/// Reads the logical fork cutoff without mistaking a revert's history base for its parent.
///
/// Older rollouts lack the explicit cutoff. Their history base is safe to use only when it
/// names the logical parent directly or the current file is the thread's original rollout.
/// An ambiguous legacy revert omits the cutoff rather than reporting another thread's boundary.
pub fn forked_from_ordinal_exclusive(
    meta: &SessionMeta,
    rollout_path: Option<&Path>,
) -> Option<u64> {
    let parent_id = meta.forked_from_id?;
    meta.forked_from_ordinal_exclusive.or_else(|| {
        meta.history_base
            .filter(|base| {
                base.thread_id == parent_id
                    || rollout_path.and_then(rollout_id_from_path) == Some(meta.id)
            })
            .map(|base| base.end_ordinal_exclusive)
    })
}
