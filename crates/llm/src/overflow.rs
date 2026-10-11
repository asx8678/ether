// Modified from prime-agent (crates/pa-ai/src/utils_inner/overflow.rs, MIT, Copyright (c) 2025-2026 Prime Intellect Ltd.; see THIRD_PARTY_NOTICES.md): the provider pattern table and the rate-limit deny-list, on regex-lite.
//! Recognises a provider's "request does not fit the context window" error, whatever its
//! wording, so the engine can compact and retry instead of failing the turn.

use regex_lite::Regex;
use std::sync::OnceLock;

/// How providers word a context overflow.
const OVERFLOW_PATTERNS: [&str; 24] = [
    r"prompt is too long",                             // Anthropic token overflow
    r"request_too_large",                              // Anthropic byte size (413)
    r"input is too long for requested model",          // Amazon Bedrock
    r"exceeds the context window",                     // OpenAI
    r"(?i)input token count.*exceeds the maximum",     // Google Gemini
    r"(?i)maximum prompt length is \d+",               // xAI
    r"(?i)reduce the length of the messages",          // Groq
    r"(?i)maximum context length is \d+ tokens",       // OpenRouter
    r"(?i)exceeds the model's maximum context length", // LiteLLM
    r"(?i)exceeds the limit of \d+",                   // GitHub Copilot
    r"(?i)exceeds the available context size",         // llama.cpp server
    r"(?i)greater than the context length",            // LM Studio
    r"(?i)context window exceeds limit",               // MiniMax
    r"(?i)exceeded model token limit",                 // Kimi
    r"(?i)too large for model with \d+ maximum context length", // Mistral
    r"(?i)model_context_window_exceeded",              // Z.AI (GLM), also a finish_reason
    r"(?i)combined input and output tokens",           // combined ceilings
    r"(?i)accepts at most \d+ combined",               // "accepts at most N combined tokens"
    r"(?i)reduce the input length or requested output length", // combined-limit remedy text
    r"(?i)prompt too long; exceeded (?:max )?context length", // Ollama
    r"(?i)context[_ ]length[_ ]exceeded",              // generic
    r"(?i)too many tokens",                            // generic
    r"(?i)token limit exceeded",                       // generic
    r"(?i)maximum context",                            // generic (ether)
];

/// Errors that match an overflow pattern but are not overflows (rate limits).
const NOT_OVERFLOW_PATTERNS: [&str; 3] = [
    r"(?i)^(Throttling error|Service unavailable):",
    r"(?i)rate limit",
    r"(?i)too many requests",
];

fn compiled(patterns: &'static [&'static str]) -> Vec<Regex> {
    patterns
        .iter()
        .filter_map(|pattern| Regex::new(pattern).ok())
        .collect()
}

/// Whether an error message or code says the request does not fit the context window.
pub(crate) fn is_overflow(text: &str) -> bool {
    static OVERFLOW: OnceLock<Vec<Regex>> = OnceLock::new();
    static NOT_OVERFLOW: OnceLock<Vec<Regex>> = OnceLock::new();
    let not_overflow = NOT_OVERFLOW.get_or_init(|| compiled(&NOT_OVERFLOW_PATTERNS));
    if not_overflow.iter().any(|pattern| pattern.is_match(text)) {
        return false;
    }
    OVERFLOW
        .get_or_init(|| compiled(&OVERFLOW_PATTERNS))
        .iter()
        .any(|pattern| pattern.is_match(text))
}
