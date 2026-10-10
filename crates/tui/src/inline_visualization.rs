// Modified from OpenAI Codex (codex-rs/tui/src/inline_visualization.rs @ 806d973): only the types and functions the lifted markdown and streaming code name, with visualizations never enabled; ether has no inline visualization viewer.
//! Inline visualization directives are not rendered: markdown passes through unchanged.

use std::borrow::Cow;
use std::collections::HashMap;
use url::Url;

pub(crate) const LINK_PLACEHOLDER_PREFIX: &str = "https://codex.invalid/inline-visualization/";

/// Where visualization documents would be resolved. Never constructed in ether.
#[derive(Clone, Debug)]
pub(crate) enum InlineVisualizationContext {}

pub(crate) struct InlineVisualizationRewrite<'a> {
    pub(crate) markdown: Cow<'a, str>,
    pub(crate) trusted_file_links: HashMap<String, TrustedFileLink>,
}

pub(crate) struct TrustedFileLink {
    pub(crate) destination: Url,
    pub(crate) markdown_label: String,
    pub(crate) display_label: String,
    pub(crate) markdown_destination_label: String,
}

pub(crate) fn contains_inline_visualization(_markdown: &str) -> bool {
    false
}

pub(crate) fn rewrite_inline_visualizations<'a>(
    markdown: &'a str,
    _context: Option<&InlineVisualizationContext>,
) -> InlineVisualizationRewrite<'a> {
    InlineVisualizationRewrite {
        markdown: Cow::Borrowed(markdown),
        trusted_file_links: HashMap::new(),
    }
}
