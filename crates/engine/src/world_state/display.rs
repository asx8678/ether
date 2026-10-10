// New in ether: the `<terminal_display>` section, after the shape of the other world-state sections.
//! What the front end can draw, in its own words (`EngineConfig::display_instructions`). The
//! text is fixed for a session, so it is sent once and stays in the cached prefix. (A resumed
//! session starts without a world-state snapshot, so, like the other sections, it is sent
//! again in full after the resumed history.)

use super::WorldStateSection;
use crate::config::EngineConfig;
use codex_context_fragments::ContextualUserFragment;
use codex_protocol::models::ContentItemKind;

const OPEN_TAG: &str = "<terminal_display>";
const CLOSE_TAG: &str = "</terminal_display>";
const NO_VIEWS: &str = "This terminal no longer draws views; answer in plain markdown.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DisplayState {
    instructions: Option<String>,
}

impl DisplayState {
    pub(crate) fn new(config: &EngineConfig) -> Self {
        Self {
            instructions: config.display_instructions.clone(),
        }
    }
}

impl WorldStateSection for DisplayState {
    type Snapshot = DisplayState;

    fn render_diff(
        &self,
        previous: Option<&Self::Snapshot>,
    ) -> (Self::Snapshot, Vec<Box<dyn ContextualUserFragment + Send>>) {
        let text = match (previous, &self.instructions) {
            (Some(previous), _) if previous == self => None,
            (_, Some(text)) => Some(text.clone()),
            (Some(_), None) => Some(NO_VIEWS.to_string()),
            (None, None) => None,
        };
        let fragment = text.map(|text| {
            Box::new(DisplayInstructions { text }) as Box<dyn ContextualUserFragment + Send>
        });
        (self.clone(), fragment.into_iter().collect())
    }
}

struct DisplayInstructions {
    text: String,
}

impl ContextualUserFragment for DisplayInstructions {
    fn role(&self) -> &'static str {
        "developer"
    }

    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("terminal_display".to_string())
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        (OPEN_TAG, CLOSE_TAG)
    }

    fn body(&self) -> String {
        format!("\n{}\n", self.text.trim())
    }
}
