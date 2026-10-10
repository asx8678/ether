// Modified from OpenAI Codex (codex-rs/core/src/context/world_state/environment.rs and core/src/context/environment_context.rs @ 806d973): one local environment (cwd, shell, current date) in the legacy single-environment rendering; no remote environments, shell versions, timezone, network, filesystem or subagents.
//! The `<environment_context>` section. The date appears only here: when the day changes, the
//! diff is a short fragment carrying just the new `<current_date>`.

use super::WorldStateSection;
use crate::config::EngineConfig;
use codex_context_fragments::ContextualUserFragment;
use codex_protocol::models::ContentItemKind;
use codex_protocol::protocol::ENVIRONMENT_CONTEXT_CLOSE_TAG;
use codex_protocol::protocol::ENVIRONMENT_CONTEXT_OPEN_TAG;
use codex_shell_command::shell_detect::default_user_shell;

/// Environment values visible to the model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EnvironmentState {
    cwd: String,
    shell: String,
    current_date: String,
}

impl EnvironmentState {
    pub(crate) fn capture(config: &EngineConfig) -> Self {
        Self {
            cwd: config.cwd.as_path().display().to_string(),
            shell: default_user_shell().name().to_string(),
            current_date: chrono::Local::now().format("%Y-%m-%d").to_string(),
        }
    }
}

impl WorldStateSection for EnvironmentState {
    type Snapshot = EnvironmentState;

    fn render_diff(
        &self,
        previous: Option<&Self::Snapshot>,
    ) -> (Self::Snapshot, Vec<Box<dyn ContextualUserFragment + Send>>) {
        let environment_changed = previous
            .is_none_or(|previous| previous.cwd != self.cwd || previous.shell != self.shell);
        let date_changed =
            previous.is_none_or(|previous| previous.current_date != self.current_date);
        let fragment = (environment_changed || date_changed).then(|| {
            Box::new(RenderedEnvironment {
                environment: environment_changed.then(|| (self.cwd.clone(), self.shell.clone())),
                current_date: self.current_date.clone(),
            }) as Box<dyn ContextualUserFragment + Send>
        });
        (self.clone(), fragment.into_iter().collect())
    }
}

struct RenderedEnvironment {
    /// `(cwd, shell)`, when they are new to the model.
    environment: Option<(String, String)>,
    current_date: String,
}

impl ContextualUserFragment for RenderedEnvironment {
    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("environments.environment_context".to_string())
    }

    fn role(&self) -> &'static str {
        "user"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        (ENVIRONMENT_CONTEXT_OPEN_TAG, ENVIRONMENT_CONTEXT_CLOSE_TAG)
    }

    fn body(&self) -> String {
        let mut rendered = "\n".to_string();
        if let Some((cwd, shell)) = &self.environment {
            push_element(&mut rendered, "cwd", cwd);
            push_element(&mut rendered, "shell", shell);
        }
        push_element(&mut rendered, "current_date", &self.current_date);
        rendered
    }
}

fn push_element(rendered: &mut String, name: &str, value: &str) {
    rendered.push_str("  <");
    rendered.push_str(name);
    rendered.push('>');
    push_xml_escaped_text(rendered, value);
    rendered.push_str("</");
    rendered.push_str(name);
    rendered.push_str(">\n");
}

fn push_xml_escaped_text(rendered: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '&' => rendered.push_str("&amp;"),
            '<' => rendered.push_str("&lt;"),
            '>' => rendered.push_str("&gt;"),
            '"' => rendered.push_str("&quot;"),
            '\'' => rendered.push_str("&apos;"),
            _ => rendered.push(ch),
        }
    }
}
