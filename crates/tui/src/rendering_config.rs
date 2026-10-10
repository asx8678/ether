// Modified from OpenAI Codex (codex-rs/config/src/tui_rendering.rs @ 806d973): local copy (codex-config is not copied); Mermaid rendering defaults to off because the Mermaid renderer is not copied.
//! TUI content rendering preferences, independent of animation effects.

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

/// Optional rich renderers. Disabled renderers preserve the original source.
#[derive(Serialize, Deserialize, Debug, Copy, Clone, PartialEq, Eq, JsonSchema)]
#[serde(default)]
#[schemars(deny_unknown_fields)]
pub struct TuiRendering {
    /// Render Mermaid code blocks as diagrams.
    pub mermaid: bool,
    /// Render math expressions using Unicode notation.
    pub math: bool,
    /// Render pipe tables, including tables inside Markdown fences.
    pub tables: bool,
    /// Render Markdown bullets and task-list markers as Unicode symbols.
    pub lists: bool,
}

impl Default for TuiRendering {
    fn default() -> Self {
        Self {
            mermaid: false,
            math: true,
            tables: true,
            lists: true,
        }
    }
}
