// Modified from OpenAI Codex (codex-rs/tools/src/tool_discovery.rs @ 806d973): removed unit tests; removed plugin and connector discovery (only tool-search items remain).
pub const TOOL_SEARCH_TOOL_NAME: &str = "tool_search";
pub const TOOL_SEARCH_DEFAULT_LIMIT: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolSearchSourceInfo {
    pub name: String,
    pub description: Option<String>,
}
