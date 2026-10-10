// Modified from OpenAI Codex (codex-rs/core/src/unified_exec/mod.rs @ 806d973): local processes only; no exec-server, shell snapshots, network approval or plugin metrics; constants and output formatting (from tools/context.rs `ExecCommandToolOutput`) kept.
//! Unified exec: interactive processes started by `exec_command` and driven by `write_stdin`,
//! run through approvals and the platform sandbox.
//!
//! - `backend.rs`: the local process backend (spawn, stdin, output, kill).
//! - `manager.rs`: approvals, sandbox selection and retry, the per-session process store.
//! - `policy.rs` and `approval.rs`: the approval decision logic.

mod approval;
mod backend;
mod buffer;
mod manager;
mod policy;
mod shell;

pub(crate) use manager::CallContext;
pub(crate) use manager::ExecCommandParams;
pub(crate) use manager::ExecManager;
pub(crate) use manager::WriteStdinParams;

use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::formatted_truncate_text;
use codex_utils_output_truncation::truncate_text;
use std::time::Duration;

pub(crate) const MIN_YIELD_TIME_MS: u64 = 250;
/// Minimum yield time for an empty `write_stdin`.
pub(crate) const MIN_EMPTY_YIELD_TIME_MS: u64 = 5_000;
pub(crate) const MAX_YIELD_TIME_MS: u64 = 30_000;
pub(crate) const MAX_BACKGROUND_TERMINAL_TIMEOUT_MS: u64 = 300_000;
pub(crate) const DEFAULT_MAX_OUTPUT_TOKENS: usize = 10_000;
pub(crate) const OUTPUT_MAX_BYTES: usize = 1024 * 1024; // 1 MiB
pub(crate) const MAX_PROCESSES: usize = 64;

pub(crate) fn clamp_yield_time(yield_time_ms: u64) -> u64 {
    yield_time_ms.clamp(MIN_YIELD_TIME_MS, MAX_YIELD_TIME_MS)
}

pub(crate) fn format_output_omission_marker(omitted_bytes: usize) -> String {
    format!("... {omitted_bytes} bytes omitted ...")
}

pub(crate) fn generate_chunk_id() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    (0..6)
        .map(|_| format!("{:x}", rng.random_range(0..16)))
        .collect()
}

/// The result of one `exec_command` or `write_stdin` call, as Codex's `ExecCommandToolOutput`.
#[derive(Debug)]
pub(crate) struct ExecOutput {
    pub(crate) chunk_id: String,
    pub(crate) wall_time: Duration,
    /// Collected bytes, with an omission marker if the collection cap dropped any.
    pub(crate) raw_output: Vec<u8>,
    pub(crate) output_omitted_bytes: usize,
    pub(crate) process_id: Option<i32>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) original_token_count: usize,
    pub(crate) max_output_tokens: Option<usize>,
}

impl ExecOutput {
    fn model_output_policy(&self, truncation_policy: TruncationPolicy) -> TruncationPolicy {
        let requested =
            TruncationPolicy::Tokens(self.max_output_tokens.unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS));
        if requested.byte_budget() < truncation_policy.byte_budget() {
            requested
        } else {
            truncation_policy
        }
    }

    fn truncated_output(&self, policy: TruncationPolicy) -> String {
        let text = String::from_utf8_lossy(&self.raw_output).to_string();
        if self.output_omitted_bytes == 0 {
            return formatted_truncate_text(&text, policy);
        }
        let marker = format_output_omission_marker(self.output_omitted_bytes);
        if text.len() <= policy.byte_budget() {
            return text;
        }
        let truncated = truncate_text(&text, policy);
        let omission_notice = if truncated.contains(&marker) {
            String::new()
        } else {
            format!("{marker}\n")
        };
        format!(
            "Warning: truncated output (original token count: {})\n{omission_notice}\n{truncated}",
            self.original_token_count
        )
    }

    fn response_header(&self) -> String {
        let mut sections = Vec::new();
        if !self.chunk_id.is_empty() {
            sections.push(format!("Chunk ID: {}", self.chunk_id));
        }
        let wall_time_seconds = self.wall_time.as_secs_f64();
        sections.push(format!("Wall time: {wall_time_seconds:.4} seconds"));
        if let Some(exit_code) = self.exit_code {
            sections.push(format!("Process exited with code {exit_code}"));
        }
        if let Some(process_id) = self.process_id {
            sections.push(format!("Process running with session ID {process_id}"));
        }
        sections.push(format!(
            "Original token count: {}",
            self.original_token_count
        ));
        sections.push("Output:".to_string());
        sections.join("\n")
    }

    /// The text the model sees, truncated by the model's policy and the call's token budget.
    pub(crate) fn response_text(&self, truncation_policy: TruncationPolicy) -> String {
        let output = self.truncated_output(self.model_output_policy(truncation_policy));
        format!("{}\n{output}", self.response_header())
    }
}
