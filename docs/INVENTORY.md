# Phase 0 inventory

A read-only survey of the Codex checkout that `ether` copies from. Nothing in `CODEX_SRC` was modified.

## Source pin

- `CODEX_SRC`: `/Users/adam2/projects/project1/codex`.
- Pinned commit: `806d9732c974bc8a51b8317c1bd8985544fe627c` (2026-10-10, "Report exec runtime resets before accepting further requests (#52825)"). It is `c3d3b14` plus one commit that only touches `code-mode` and `code-mode-host`, both on the never-copy list, so the facts verified at `c3d3b14` still apply.
- Analysis and copying use `git archive 806d973 codex-rs LICENSE NOTICE`, extracted to `~/.cache/ether/codex-806d973`. Cargo never runs inside `CODEX_SRC`.
- Toolchain: Rust 1.95.0 (`codex-rs/rust-toolchain.toml`), edition 2024, licence Apache-2.0.

**Line counts** are non-test lines: raw lines of `.rs` files, excluding crate-relative `tests/` and `benches/` directories, `*_tests.rs` and `tests.rs` files, and `#[cfg(test)]` items. They differ from your figures by 5–25%, which looks like a different counting method. No crate is unexpectedly large.

## 1. Facts check

| Claim | Found at 806d973 | |
|---|---|---|
| About 155 crates | 160 workspace members, 6 of them test-support crates | ≈ |
| About 916k non-test lines | 868k (2.04M including tests) | ≈ |
| `codex-core`: 114k lines, 69 internal dependencies | 119.7k lines; 69 direct internal dependencies (none optional), 11 internal dev-dependencies | ✓ |
| Core's dependency closure is 92 crates | 92 | ✓ |
| `codex-tui`: 246k lines | 214.9k | ≈ |
| The TUI reaches core only through `app-server-client` → `app-server` → core | The TUI has no direct core dependency. `app-server-client` (closure of 117 crates) also depends on core directly; `app-server` is 45.4k lines | ✓ |
| Only the Responses API; `wire_api = "chat"` is rejected | `model-provider-info/src/lib.rs:107` (`enum WireApi { Responses }`); `:100` and `:130` (`CHAT_WIRE_API_REMOVED_ERROR`) | ✓ |
| The loop consumes `ResponseEvent` | `codex-api/src/common.rs:81` | ✓ |
| `ApplyPatchToolType` only has `Freeform` | `protocol/src/openai_models.rs:326` | ✓ |
| Models outside the catalog: no `apply_patch`, 272k context, 10 KB truncation | `models-manager/src/model_info.rs:124-129` | ✓ |
| `ProviderCapabilities::default()` turns on web search and image generation | `model-provider/src/capabilities.rs:19-28`. `from_config` lets config override only `external_web_access` and `remote_compaction`, so a provider config cannot turn them off | ✓ (stronger) |
| apply-patch uses exec-server only for its filesystem; the trait is in `codex-file-system` | apply-patch imports seven types that exec-server re-exports from `codex-file-system`, plus `LOCAL_FS`. The trait is at `file-system/src/lib.rs:639` (11 methods). **The local implementation, `LOCAL_FS`, lives in exec-server, not file-system** (see C2) | ✓ with caveat |
| TUI: about 50 files use the app-server client and about 64 use config | 49 and 69 non-test files | ✓ |

## 2. Copy list, in copy order

Each crate's internal dependencies are either earlier in this list or listed under its cuts. Package names are `codex-<dir>` or `codex-utils-<dir>` unless shown.

### Tier 1: 20 crates, 13.5k lines, no source cuts

| # | Directory | Lines | Internal deps | Notes |
|---|---|---|---|---|
| 1 | `utils/absolute-path` | 528 | – | |
| 2 | `utils/string` | 379 | – | |
| 3 | `utils/stream-parser` | 905 | – | |
| 4 | `utils/fuzzy-match` | 65 | – | |
| 5 | `utils/cache` | 165 | – | |
| 6 | `utils/process` | 23 | – | |
| 7 | `utils/redacted-string` | 49 | – | |
| 8 | `async-utils` | 56 | – | |
| 9 | `ansi-escape` | 63 | – | ratatui and ansi-to-tui pull in crossterm (C5) |
| 10 | `file-search` | 792 | – | |
| 11 | `process-hardening` | 147 | – | |
| 12 | `uds` | 781 | – | |
| 13 | `bwrap` plus `vendor/bubblewrap` | 151 + C | – | `build.rs` compiles bubblewrap on Linux only; macOS gets a stub |
| 14 | `utils/pty` | 4,652 | – | dev-dependency `utils/cargo-bin` removed (C6) |
| 15 | `utils/path-uri` | 1,801 | absolute-path | |
| 16 | `utils/home-dir` | 64 | absolute-path | |
| 17 | `utils/path-utils` (`codex-utils-path`) | 421 | absolute-path | |
| 18 | `ext/items` (`codex-extension-items`) | 172 | absolute-path | see C1 |
| 19 | `execpolicy` | 1,808 | absolute-path | uses starlark |
| 20 | `utils/image` | 522 | cache | uses image and png |

### Tier 2: 14 crates, 62.7k lines

Cuts are given as references / files, counting non-test code only.

| # | Crate | Lines | Internal deps kept | Cuts |
|---|---|---|---|---|
| 21 | `protocol` | 22,681 | async-utils, execpolicy, extension-items, utils: absolute-path, image, path-uri, redacted-string, string | `network-proxy` 3/2: local copies of `EnvironmentNetworkPolicy`, `NetworkDecisionSource`, `NetworkPolicyDecision`. `http-client` 2/1: `HttpError`, `RetryAfter`. Realtime, guardian and cyber-access types: `realtime.rs` (59), `guardian_transcript.rs` (16), and mentions in 14 files |
| 22 | `utils/output-truncation` | 262 | protocol, string | – |
| 23 | `file-system` | 1,152 | protocol, absolute-path, path-uri | none; gains a local implementation (C2) |
| 24 | `shell-command` | 5,531 | protocol, absolute-path | candidate: `tree-sitter-powershell` (Windows shells) |
| 25 | `history` | 2,308 | protocol | – |
| 26 | `context-fragments` | 469 | protocol, string | – |
| 27 | `git-utils` | 3,379 | file-system, protocol, absolute-path, path-uri, process, pty | – |
| 28 | `apply-patch` | 2,828 | absolute-path, path-uri, plus file-system | `exec-server` 10/3, replaced by `codex-file-system` |
| 29 | `codex-client` | 300 | – | `http-client` 9/4 (`HttpClient`, `Request`, `RequestBuilder`, `ByteStream`, `StreamError`, `TransportError`, `RetryAfter`), replaced by plain `reqwest` |
| 30 | `model-provider-info` | 963 | codex-client, protocol, redacted-string | Bedrock and ChatGPT constants (`lib.rs:79-97`); add `WireApi::ChatCompletions` and drop the `"chat"` rejection |
| 31 | `sandboxing` | 4,454 | protocol, uds, pty, absolute-path, path-uri | `mxc-sandbox` 7/3, `network-proxy` 10/4, `windows-sandbox` 11/4, `otel` 1/1 (Windows only) |
| 32 | `linux-sandbox` | 5,233 | sandboxing, process-hardening, protocol, uds, absolute-path (all Linux only) | `install-context` 4/1, `network-proxy` 4/3 |
| 33 | `tools` | 3,365 | extension-items, file-system, protocol, output-truncation, path-uri, string | `code-mode` 14/2, `connectors` 2/2, `features` 2/1; candidate: external `rmcp` (MCP types) |
| 34 | `rollout` | 9,820 | extension-items, file-search, git-utils, history, output-truncation, path, process, string, protocol | `otel` 13/7, `state` 81/7 (C3) |

`history` (`codex-history`) holds model-history and rollout domain types, as your list says. The `~/.codex/history.jsonl` prompt history is a different crate, `message-history`, which is not on the list. The TUI's up-arrow history would need it in Phase 5.

## 3. Conflicts and surprises

- **C1. `extension-items`.** It is in Tier 1, but its directory `ext/items` matches the never-copy pattern `ext/*`, and your table cuts it from rollout. It is used by protocol (3 references), tools (1) and rollout (1). It is 172 lines and depends only on absolute-path. *Proposal: keep it, and don't cut it from rollout.*
- **C2. The local filesystem is in exec-server.** `file-system` holds only the trait. `LOCAL_FS` is `exec-server/src/local_file_system.rs` (1,423 lines) plus `local_file_system_read.rs` (74). Those files need only `codex-file-system`, absolute-path, path-uri, tokio-util and `ExecServerRuntimeOptions` (82 lines). *Proposal: copy the two files into `crates/file-system` as its local implementation, with a small options struct in place of `ExecServerRuntimeOptions`.* The alternative is a new `tokio::fs` implementation of the 11-method trait.
- **C3. The rollout → state cut has 81 call sites in 7 files**, over the 30-site limit. 45 of them are in two files that can be deleted whole: `state_db.rs` (744 lines, 41 references) and `sqlite_metrics.rs` (73, 4). The other 36 are in `recorder.rs` (11), `list.rs` (11), `metadata.rs` (11), `session_index.rs` (2) and `config.rs` (1). *Proposal: delete the two SQLite files, drop `metadata.rs` if it only feeds SQLite, and keep listing and replay working on JSONL files alone (`list_files.rs`, `reverse_jsonl_scanner.rs`).*
- **C4. The engine's exec path also goes through exec-server** (Phase 3). `core/src/unified_exec` makes 19 references to `Environment`, `ExecParams`, `ExecProcess`, `StartedExecProcess`, `ExecProcessEvent`, `ShellSnapshotRequest` and other types. The local backend behind them is large: `local_process.rs` (2,205 lines), `environment.rs` (2,046), `process.rs` (317), and about 730 lines of shell snapshots. *Proposal for Phase 3: copy only the process trait with its parameter and event types, write a slim local backend on `codex-utils-pty`, and leave shell snapshots out at first.*
- **C5. crossterm fork.** `ansi-escape` (Tier 1) pulls in ratatui and so crossterm. Codex patches crossterm to `openai-oss-forks/crossterm@ed1cdab`. *Proposal: carry that one `[patch.crates-io]` entry from Phase 1, since the TUI terminal code will probably need it in Phase 5, and drop the two tungstenite patches.*
- **C6. Dev-dependencies on crates not on the list.** `codex-utils-cargo-bin` is a dev-dependency of pty, apply-patch, sandboxing, git-utils, tools and linux-sandbox; `codex-core` is one of linux-sandbox. Cargo resolves dev-dependencies even when tests never run, so their paths would have to exist. *Proposal: remove those dev-dependency lines, since tests are off.*

## 4. Tier 3 sources

All the listed files exist.

- `exec_command` and `write_stdin` are `tools/handlers/unified_exec.rs` plus `tools/handlers/unified_exec/`.
- `update_plan` is `tools/handlers/plan.rs`.
- Tool specs are in the `*_spec.rs` files next to each handler.
- `chat_composer.rs` is at `bottom_pane/chat_composer.rs` and is not copied.

| Group | Lines | Main couplings to cut or replace |
|---|---|---|
| engine: turn loop (`session/turn.rs` 3,206, `stream_events_utils.rs`, `tasks/mod.rs`, `tasks/regular.rs`) | 4,940 | guardian, hook runtime, plugins, otel, analytics, skills, connectors, extension-api, thread-store, memories, state, mcp. Target is about 300 lines |
| engine: steering (`input_queue.rs`, `turn_input.rs`) | 1,484 | `crate::state`, agent communication |
| engine: tool pipeline (registry, router, orchestrator, approvals, sandboxing, parallel, runtimes) | 5,626 | network-proxy (18), guardian (19), otel, extension-api, hooks, features, code-mode |
| engine: handlers (unified_exec, apply_patch, view_image, plan, request_user_input, and their specs) | 6,846 | exec-server (C4), features, core-plugins, network-proxy |
| engine: context (`context_manager/{history,normalize}`, `context_window`, `compact.rs`, `world_state` environment, permissions and AGENTS.md sections) | 4,266 | analytics, extension-api, prompts, guardian-context, config |
| engine: other (`responses_retry`, `exec_policy`, `shell`, `agents_md`, `turn_diff_tracker`, `rollout_reconstruction`) | 3,114 | config (7), home (6), exec-server (4), http-client (1) |
| llm: `codex-api` (`common.rs`, `sse/responses*.rs`, `endpoint/responses.rs`, `requests/`, `error.rs`, `rate_limits.rs`) | 1,781 | codex-client (kept), http-client (2) |
| tui: terminal (`custom_terminal`, `insert_history`, `wrapping`) | 2,730 | no Codex crates; small internal modules (`terminal_hyperlinks`, `render`, `width`, `line_truncation`) |
| tui: markdown (`markdown_render`, `markdown_stream`, `streaming/`, `render/highlight`) | 5,857 | `codex_home` (24 references, file-link rendering), internal helpers (`markdown_copy`, `table_detect`, `terminal_hyperlinks`) |
| tui: diff, textarea, paste burst, approval overlay | 5,278 | app-server-protocol (13), keymap, terminal-detection |
| tui: `history_cell/`, `exec_cell/` | about 8,200 | app-server-protocol (29), config, otel, motion, daybreak and local-settings helpers |

Tier 3 code also references crates that are on neither list. They will be replaced locally rather than copied:

- `codex-prompts` (compaction prompt templates)
- `codex-home` (resolving the Codex home directory)
- `codex-model-provider` (capabilities)
- `app-server-protocol`: the TUI cells and approval overlay use its types, which become our `EventMsg` and item types
- `models-manager`: only `prompt.md` (275 lines) is copied

## 5. External crates

Tiers 1 and 2 use 83 external crates. All of them are in Codex's `Cargo.lock`, so they count as pre-approved. The heavy ones:

- starlark (execpolicy)
- gix (git-utils)
- tree-sitter with the bash and powershell grammars (shell-command, apply-patch)
- image and png (utils/image)
- schemars and ts-rs derives (protocol and others)
- icu (protocol)
- rmcp (tools)
- ratatui and ansi-to-tui (ansi-escape)
- portable-pty (pty)
- landlock and seccompiler (linux-sandbox, Linux only)
- zstd (rollout)

Windows-only dependencies are dropped.

## 6. Working defaults

- Target: `/Users/adam2/projects/project1/ether`, workspace `ether`. New crates are `ether-engine`, `ether-llm`, `ether-tui` and `ether-cli`; copied crates keep their `codex-*` names.
- Workspace: start from Codex's `Cargo.lock`, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`, `[workspace.lints]` (38 clippy entries), and its dev and release profiles.
- Layout: `crates/<codex dir>`, keeping `utils/`. `vendor/bubblewrap` goes at `crates/vendor/bubblewrap` so `bwrap/build.rs` finds it unchanged. `ext/items` becomes `crates/extension-items`.
- Platforms: Linux and macOS only; Windows-only code and dependencies are removed. Linux-only code stays behind its `cfg` and will be checked on a Linux machine later.
- **No tests.** None are written, copied or run. Copied crates lose all test code: `#[cfg(test)]` items, `*_tests.rs` files, `tests/`, `benches/`, insta `snapshots/` and dev-dependencies. Every step's gate is `cargo check --workspace`; from Phase 3 on, the check is building and running the binary. The test gates in Phases 1–6 (mock SSE tests, the prompt-cache prefix test, insta snapshots) are dropped.

## 7. Resolved

1. C1, C2, C3, C5 and C6 are approved; C4 will be decided at Phase 3.
2. Unit tests are not copied at all; test code is stripped from copied crates.
3. Running the built binary is the check.
