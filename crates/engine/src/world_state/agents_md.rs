// Modified from OpenAI Codex (codex-rs/core/src/context/world_state/agents_md.rs and core/src/context/user_instructions.rs @ 806d973): typed previous snapshot only (no legacy-fragment matching); loads through ether's local AGENTS.md discovery.
//! The AGENTS.md section. A change re-sends the instructions with a notice that they replace
//! the earlier ones; removal sends a short notice.

use super::WorldStateSection;
use crate::agents_md::LoadedAgentsMd;
use crate::config::EngineConfig;
use codex_context_fragments::ContextualUserFragment;
use codex_protocol::models::ContentItemKind;

const REPLACEMENT_NOTICE: &str =
    "These AGENTS.md instructions replace all previously provided AGENTS.md instructions.";
const REMOVAL_NOTICE: &str = "The previously provided AGENTS.md instructions no longer apply.";

/// The AGENTS.md instructions currently visible to the model.
#[derive(Clone, Debug, Default)]
pub(crate) struct AgentsMdState {
    instructions: Option<UserInstructions>,
}

/// Model-visible AGENTS.md state, without filesystem provenance.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AgentsMdSnapshot {
    directory: Option<String>,
    text: Option<String>,
}

impl AgentsMdState {
    pub(crate) fn load(config: &EngineConfig) -> Self {
        let loaded = LoadedAgentsMd::load(&config.ether_home, config.cwd.as_path());
        Self {
            instructions: loaded.map(|loaded| UserInstructions {
                directory: loaded.directory(),
                text: loaded.text(),
            }),
        }
    }
}

impl WorldStateSection for AgentsMdState {
    type Snapshot = AgentsMdSnapshot;

    fn render_diff(
        &self,
        previous: Option<&Self::Snapshot>,
    ) -> (Self::Snapshot, Vec<Box<dyn ContextualUserFragment + Send>>) {
        let current = match &self.instructions {
            Some(instructions) => AgentsMdSnapshot {
                directory: instructions.directory.clone(),
                text: Some(instructions.text.clone()),
            },
            None => AgentsMdSnapshot::default(),
        };
        if previous == Some(&current) {
            return (current, Vec::new());
        }

        let previous_may_contain_instructions =
            previous.is_some_and(|previous| previous.text.is_some());
        let instructions = match (&self.instructions, previous_may_contain_instructions) {
            (Some(instructions), true) => UserInstructions {
                directory: instructions.directory.clone(),
                text: format!("{REPLACEMENT_NOTICE}\n\n{}", instructions.text),
            },
            (Some(instructions), false) => instructions.clone(),
            (None, true) => UserInstructions {
                directory: None,
                text: REMOVAL_NOTICE.to_string(),
            },
            (None, false) => return (current, Vec::new()),
        };
        (current, vec![Box::new(instructions)])
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UserInstructions {
    pub(crate) directory: Option<String>,
    pub(crate) text: String,
}

impl ContextualUserFragment for UserInstructions {
    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("agents_md.instructions".to_string())
    }

    fn role(&self) -> &'static str {
        "user"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("# AGENTS.md instructions", "</INSTRUCTIONS>")
    }

    fn body(&self) -> String {
        let directory = self
            .directory
            .as_ref()
            .map(|directory| format!(" for {directory}"))
            .unwrap_or_default();
        format!("{directory}\n\n<INSTRUCTIONS>\n{}\n", self.text)
    }
}
