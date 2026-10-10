// Modified from OpenAI Codex (codex-rs/tui/src/markdown_render/preferences.rs @ 806d973): removed unit tests.
//! Content preferences shared by streaming and completed Markdown renderers.
//!
//! Seed before startup previews, then refresh when resolved session settings become active.
//! Tests isolate preferences per thread so parallel renderer tests cannot affect one another.

use crate::rendering_config::TuiRendering;

#[cfg(not(test))]
static RENDERING: std::sync::LazyLock<std::sync::RwLock<TuiRendering>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(TuiRendering::default()));

pub(crate) fn init(rendering: TuiRendering) {
    #[cfg(not(test))]
    {
        *RENDERING
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = rendering;
    }
}

pub(crate) fn current() -> TuiRendering {
    #[cfg(not(test))]
    {
        *RENDERING
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
