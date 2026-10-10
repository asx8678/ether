// Modified from OpenAI Codex (codex-rs/core/src/context/turn_aborted.rs @ 806d973): the user-role interrupted-turn marker only.
//! Model-visible context fragments the engine injects into history.

use codex_context_fragments::ContextualUserFragment;
use codex_protocol::models::ContentItemKind;

/// Tells the model that the previous turn was interrupted on purpose.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TurnAborted {
    pub(crate) guidance: String,
}

impl TurnAborted {
    pub(crate) const INTERRUPTED_GUIDANCE: &'static str = "The user interrupted the previous turn on purpose. Any running unified exec processes may still be running in the background. If any tools/commands were aborted, they may have partially executed.";

    pub(crate) fn interrupted() -> Self {
        Self {
            guidance: Self::INTERRUPTED_GUIDANCE.to_string(),
        }
    }
}

impl ContextualUserFragment for TurnAborted {
    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("generic.turn_aborted".to_string())
    }

    fn role(&self) -> &'static str {
        "user"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("<turn_aborted>", "</turn_aborted>")
    }

    fn body(&self) -> String {
        format!("\n{}\n", self.guidance)
    }
}
