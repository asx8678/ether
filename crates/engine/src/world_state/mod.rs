// Modified from OpenAI Codex (codex-rs/core/src/context/world_state/mod.rs and core/src/context_manager/updates.rs @ 806d973): three typed sections compared by value; no extension sections, placement, hashing, merge patches or legacy/retained fragment matching.
//! The model-visible "world state": environment, permissions and AGENTS.md instructions.
//!
//! Each section renders itself relative to the snapshot the model last saw. The first turn
//! renders everything; later turns append only what changed, so earlier items (and the
//! prompt-cache prefix they form) are never rewritten.

mod agents_md;
mod environment;
mod permissions;

use crate::config::EngineConfig;
use codex_context_fragments::ContextualUserFragment;
use codex_context_fragments::RenderedFragment;
use codex_protocol::models::InternalChatMessageMetadataPassthrough;
use codex_protocol::models::ResponseItem;

pub(crate) use agents_md::AgentsMdState;
pub(crate) use environment::EnvironmentState;
pub(crate) use permissions::PermissionsState;

/// A typed portion of the state visible to the model.
pub(crate) trait WorldStateSection {
    type Snapshot: Clone + PartialEq;

    /// Returns the current snapshot and the fragments that tell the model what changed since
    /// `previous` (everything when `previous` is `None`).
    fn render_diff(
        &self,
        previous: Option<&Self::Snapshot>,
    ) -> (Self::Snapshot, Vec<Box<dyn ContextualUserFragment + Send>>);
}

/// What the model last saw of each section.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct WorldStateSnapshot {
    agents_md: Option<<AgentsMdState as WorldStateSection>::Snapshot>,
    permissions: Option<<PermissionsState as WorldStateSection>::Snapshot>,
    environment: Option<<EnvironmentState as WorldStateSection>::Snapshot>,
}

/// Live model-visible state, in Codex's section order.
pub(crate) struct WorldState {
    agents_md: AgentsMdState,
    permissions: PermissionsState,
    environment: EnvironmentState,
}

impl WorldState {
    /// Captures the current state from configuration, the filesystem and the clock.
    pub(crate) fn capture(config: &EngineConfig) -> Self {
        Self {
            agents_md: AgentsMdState::load(config),
            permissions: PermissionsState::new(config),
            environment: EnvironmentState::capture(config),
        }
    }

    /// Renders the items that bring a model which saw `previous` up to date, and the new
    /// snapshot. `None` renders the full initial context.
    pub(crate) fn render_diff(
        &self,
        previous: Option<&WorldStateSnapshot>,
    ) -> (WorldStateSnapshot, Vec<ResponseItem>) {
        let mut fragments = Vec::new();
        let (agents_md, updates) = self
            .agents_md
            .render_diff(previous.and_then(|previous| previous.agents_md.as_ref()));
        fragments.extend(updates);
        let (permissions, updates) = self
            .permissions
            .render_diff(previous.and_then(|previous| previous.permissions.as_ref()));
        fragments.extend(updates);
        let (environment, updates) = self
            .environment
            .render_diff(previous.and_then(|previous| previous.environment.as_ref()));
        fragments.extend(updates);
        let snapshot = WorldStateSnapshot {
            agents_md: Some(agents_md),
            permissions: Some(permissions),
            environment: Some(environment),
        };
        (snapshot, merge_fragments(fragments))
    }
}

/// Groups adjacent fragments with the same role into one message, as Codex's
/// `merge_world_state_updates` does for mergeable updates.
fn merge_fragments(fragments: Vec<Box<dyn ContextualUserFragment + Send>>) -> Vec<ResponseItem> {
    let mut items = Vec::new();
    let mut pending = Vec::<RenderedFragment>::new();
    for fragment in fragments {
        let rendered = fragment.render_fragment();
        if pending
            .first()
            .is_some_and(|previous| previous.role() != rendered.role())
        {
            items.extend(build_rendered_message(std::mem::take(&mut pending)));
        }
        pending.push(rendered);
    }
    items.extend(build_rendered_message(pending));
    items
}

fn build_rendered_message(fragments: Vec<RenderedFragment>) -> Option<ResponseItem> {
    let role = fragments.first()?.role();
    let (content, content_item_kinds): (Vec<_>, Vec<_>) = fragments
        .into_iter()
        .map(|fragment| fragment.into_parts().1.into_parts())
        .unzip();

    Some(ResponseItem::Message {
        status: None,
        encrypted_content: None,
        id: None,
        role: role.to_string(),
        content,
        phase: None,
        internal_chat_message_metadata_passthrough: Some(InternalChatMessageMetadataPassthrough {
            content_item_kinds: Some(content_item_kinds),
            ..Default::default()
        }),
    })
}
