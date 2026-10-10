# Phase 3: engine

Port Codex's turn loop, tool pipeline and context manager into new crates. Codex core's files are too coupled to copy whole (they reach into `Session`, `config`, guardian, hooks, analytics and exec-server almost everywhere), so the engine is new code shaped like Codex's, lifting functions and match arms where they stand alone.

Rules from the brief:

- The engine is driven only by `Op` in and `EventMsg` out.
- Agent logic stays close to a `step(state, event) -> {state, effects}` shape.
- No global state in the engine: every effect (events, tool execution, model calls) goes through a handle passed in, so several agents can run in one process.

Decisions for this phase are in `DECISIONS.md`. Check: `cargo build --workspace` with zero warnings, `scripts/check-linux.sh`, and running the `ether` binary.

## Setup

- [ ] Write this to-do list and the phase decisions.
- [ ] Create `ether-llm`, `ether-engine` and `ether-cli` and add them to the workspace.

## Protocol trim (deferred from Phase 2)

- [ ] Remove the realtime types and their `Op`/`EventMsg` variants (`realtime.rs`).
- [ ] Remove the guardian types and variants (`guardian_transcript.rs`, `openai_models/guardian*.rs`, `history/guardian_history.rs`).
- [ ] Remove the cyber-access types (`openai_models/access_programs.rs`).
- [ ] Remove `SandboxType::Windows*`.
- [ ] Fix the knock-on references in history and rollout.

The trim is limited to these types. Others (`Collab*`, `Review`, MCP and memory variants) stay unless you approve cutting them.

## Model client (`ether-llm`)

- [ ] Lift `ResponseEvent` and the request types from `codex-api` (`common.rs`, `error.rs`, `rate_limits.rs`).
- [ ] Lift the Responses SSE parser (`sse/responses.rs`, `sse/responses_error.rs`) and endpoint (`endpoint/responses.rs`, `requests/`): HTTP only, with no WebSocket, realtime, search, images or memories.
- [ ] A `ModelClient` trait the engine calls, implemented by the Responses client and by the replay source.
- [ ] Replay source: feed a file of recorded SSE through the same parser (`ether --replay FILE`), so the binary runs end to end before a real provider is connected in Phase 4.

## Engine skeleton (first runnable milestone)

- [ ] Slim config: `~/.ether/config.toml`, an optional project `.ether/config.toml`, and CLI overrides (model, provider, approval policy, sandbox mode).
- [ ] Engine handle: submit `Op`, receive `EventMsg`; per-session state passed in, not global.
- [ ] History manager and normalization (`context_manager/{history,normalize}.rs`).
- [ ] Turn loop (from `session/turn.rs` `run_turn` and `try_run_sampling_request`): sample, stream text and reasoning, record items, follow up after tool calls, stop.
- [ ] Item mapping from model output to turn items (`event_mapping.rs`, `stream_events_utils.rs`), producing `ItemStarted`/`ItemCompleted` for the Phase 5 TUI.
- [ ] Interrupt and abort (`tasks/mod.rs`): no tool call is left without a result.
- [ ] Steering: queue user input mid-turn (`session/input_queue.rs`, `session/turn_input.rs`).
- [ ] Base system prompt (`models-manager/prompt.md`).
- [ ] `ether` plain-text CLI: type a prompt, see the streamed answer and tool activity, answer approvals.

## Tool pipeline

- [ ] Registry, router and parallel calls (`tools/{registry,router,parallel}.rs`, `function_tool.rs`).
- [ ] `update_plan` and `request_user_input` handlers.
- [ ] C4 local process backend on `codex-utils-pty`: spawn, write stdin, read output, kill. The sandbox transform from `codex-sandboxing` wraps the command before spawn. No shell snapshots.
- [ ] `exec_command` and `write_stdin` (`unified_exec`, local path only), with `exec_env.rs`, `shell.rs`, `command_canonicalization.rs`.
- [ ] Approvals and sandboxing (`tools/{orchestrator,approvals,sandboxing}.rs`, `sandboxing/mod.rs`, `safety.rs`), and `exec_policy.rs`.
- [ ] `apply_patch` through `codex-apply-patch` and `codex-file-system`, plus `turn_diff_tracker.rs`.
- [ ] `view_image`.

## Context

- [ ] World-state sections: environment, permissions and AGENTS.md (`context/world_state/`, `agents_md.rs`).
- [ ] Context-window tracking and local-only compaction (`session/context_window.rs`, `compact.rs`).
- [ ] Retries (`responses_retry.rs`).

## Rollout

- [ ] Record sessions as JSONL through `codex-rollout`.
- [ ] Resume (`session/rollout_reconstruction.rs`).

## Check

- [ ] `cargo build --workspace` with zero warnings.
- [ ] `scripts/check-linux.sh`, extended to the new crates where they build for Linux.
- [ ] Run `ether --replay` through a turn with a shell command and a patch.
