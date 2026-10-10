// New in ether: the model facts the engine needs, in place of Codex's ModelInfo catalog schema.
//! Defaults follow Codex's fallback for unknown slugs (`models-manager/src/model_info.rs`,
//! `model_info_from_slug`): a 272k context window, 95% of it usable, and 10 KB output
//! truncation.

use codex_utils_output_truncation::TruncationPolicy;

/// What the engine needs to know about a model.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelProfile {
    pub slug: String,
    /// Full context window in tokens.
    pub context_window: i64,
    /// Share of the context window the engine fills before compacting.
    pub effective_context_window_percent: i64,
    /// How tool output is truncated before it enters history.
    pub truncation_policy: TruncationPolicy,
    /// Whether the model accepts reasoning settings.
    pub supports_reasoning: bool,
    /// Whether the model may call several tools in one response.
    pub supports_parallel_tool_calls: bool,
    /// GLM: send Z.AI's `thinking` object on Chat Completions.
    pub glm_thinking: bool,
}

impl ModelProfile {
    /// Returns the built-in profile for `slug`, or the fallback profile.
    pub fn for_slug(slug: &str) -> Self {
        if slug.starts_with("glm-") {
            return Self::glm(slug);
        }
        Self {
            slug: slug.to_string(),
            context_window: 272_000,
            effective_context_window_percent: 95,
            truncation_policy: TruncationPolicy::Bytes(10_000),
            supports_reasoning: slug.starts_with("gpt-5") || slug.starts_with('o'),
            supports_parallel_tool_calls: true,
            glm_thinking: false,
        }
    }

    /// GLM through Zro. GLM 5.3's window is Zro's live catalog value (`max_input_tokens`
    /// 1,048,576); other GLM slugs use the bundled catalog's GLM 5.2 value (524,288).
    fn glm(slug: &str) -> Self {
        let context_window = if slug.starts_with("glm-5.3") {
            1_048_576
        } else {
            524_288
        };
        Self {
            slug: slug.to_string(),
            context_window,
            effective_context_window_percent: 95,
            truncation_policy: TruncationPolicy::Bytes(10_000),
            supports_reasoning: true,
            supports_parallel_tool_calls: true,
            glm_thinking: true,
        }
    }

    /// Tokens usable before compaction.
    pub fn usable_context_window(&self) -> i64 {
        self.context_window * self.effective_context_window_percent / 100
    }
}
