# Provenance

Everything here comes from OpenAI Codex (<https://github.com/openai/codex>, Apache-2.0) at commit `806d9732c974bc8a51b8317c1bd8985544fe627c`, unless noted otherwise. Every changed file has a "Modified from OpenAI Codex" note on its first line.

Applied to every copied crate (see `DECISIONS.md`):

- unit tests, benches and dev-dependencies removed;
- Windows-only code and Windows-only dependency tables removed;
- `BUILD.bazel` not copied.

| ether path | Codex source | Changes |
|---|---|---|
| `LICENSE`, `NOTICE` | `LICENSE`, `NOTICE` | none |
| `rust-toolchain.toml`, `rustfmt.toml` | `codex-rs/rust-toolchain.toml`, `codex-rs/rustfmt.toml` | none |
| `clippy.toml` | `codex-rs/clippy.toml` | removed the sqlx rules |
| `Cargo.toml` | `codex-rs/Cargo.toml` | trimmed to the copied crates; only the `dev` and `release` profiles; only the crossterm patch |
| `crates/utils/absolute-path` (`codex-utils-absolute-path`) | `codex-rs/utils/absolute-path` | unit tests stripped from 3 file(s), 1 test file(s) deleted; Windows-only code stripped from 1 file(s); manifest: removed `[dev-dependencies]` |
| `crates/utils/string` (`codex-utils-string`) | `codex-rs/utils/string` | unit tests stripped from 3 file(s), 1 test file(s) deleted; manifest: removed `[dev-dependencies]` |
| `crates/utils/stream-parser` (`codex-utils-stream-parser`) | `codex-rs/utils/stream-parser` | unit tests stripped from 6 file(s), 0 test file(s) deleted; manifest: removed `[dev-dependencies]` |
| `crates/utils/fuzzy-match` (`codex-utils-fuzzy-match`) | `codex-rs/utils/fuzzy-match` | unit tests stripped from 1 file(s), 0 test file(s) deleted |
| `crates/utils/cache` (`codex-utils-cache`) | `codex-rs/utils/cache` | unit tests stripped from 1 file(s), 1 test file(s) deleted; manifest: removed `[dev-dependencies]` |
| `crates/utils/process` (`codex-utils-process`) | `codex-rs/utils/process` | unit tests stripped from 1 file(s), 1 test file(s) deleted; Windows-only code stripped from 1 file(s); manifest: removed `[dev-dependencies]` |
| `crates/utils/redacted-string` (`codex-utils-redacted-string`) | `codex-rs/utils/redacted-string` | none |
| `crates/async-utils` (`codex-async-utils`) | `codex-rs/async-utils` | unit tests stripped from 1 file(s), 0 test file(s) deleted; manifest: removed `[dev-dependencies]` |
| `crates/ansi-escape` (`codex-ansi-escape`) | `codex-rs/ansi-escape` | unit tests stripped from 1 file(s), 1 test file(s) deleted; manifest: removed `[dev-dependencies]` |
| `crates/file-search` (`codex-file-search`) | `codex-rs/file-search` | unit tests stripped from 1 file(s), 1 test file(s) deleted; Windows-only code stripped from 1 file(s); manifest: removed `[dev-dependencies]` |
| `crates/process-hardening` (`codex-process-hardening`) | `codex-rs/process-hardening` | unit tests stripped from 1 file(s), 0 test file(s) deleted; Windows-only code stripped from 1 file(s); manifest: removed `[dev-dependencies]` |
| `crates/uds` (`codex-uds`) | `codex-rs/uds` | unit tests stripped from 1 file(s), 1 test file(s) deleted; Windows-only code stripped from 2 file(s); deleted unreachable `src/windows_peer.rs`, `src/windows_security.rs`, `src/windows_socket_validation.rs`; manifest: removed `[target.'cfg(windows)'.dependencies]`, `[dev-dependencies]` |
| `crates/bwrap` (`codex-bwrap`) | `codex-rs/bwrap` | none |
| `crates/vendor/bubblewrap` (LGPL-2.0-or-later) | `codex-rs/vendor/bubblewrap` | `tests/` not copied |
| `crates/utils/pty` (`codex-utils-pty`) | `codex-rs/utils/pty` | unit tests stripped from 11 file(s), 16 test file(s) deleted; Windows-only code stripped from 6 file(s); deleted unreachable `src/win/conpty.rs`, `src/win/job.rs`, `src/win/mod.rs`, `src/win/procthreadattr.rs`, `src/win/psuedocon.rs`, `src/windows_input.rs`; manifest: removed `[dev-dependencies]`, `[target.'cfg(windows)'.dependencies]` |
| `crates/utils/path-uri` (`codex-utils-path-uri`) | `codex-rs/utils/path-uri` | unit tests stripped from 4 file(s), 4 test file(s) deleted; Windows-only code stripped from 3 file(s); manifest: removed `[dev-dependencies]` |
| `crates/utils/home-dir` (`codex-utils-home-dir`) | `codex-rs/utils/home-dir` | unit tests stripped from 1 file(s), 0 test file(s) deleted; manifest: removed `[dev-dependencies]` |
| `crates/utils/path-utils` (`codex-utils-path`) | `codex-rs/utils/path-utils` | unit tests stripped from 2 file(s), 2 test file(s) deleted; Windows-only code stripped from 2 file(s); manifest: removed `[target.'cfg(windows)'.dependencies]`, `[dev-dependencies]` |
| `crates/extension-items` (`codex-extension-items`) | `codex-rs/ext/items` | unit tests stripped from 1 file(s), 1 test file(s) deleted; manifest: removed `[dev-dependencies]` |
| `crates/execpolicy` (`codex-execpolicy`) | `codex-rs/execpolicy` | unit tests stripped from 2 file(s), 1 test file(s) deleted; Windows-only code stripped from 1 file(s); manifest: removed `[dev-dependencies]` |
| `crates/utils/image` (`codex-utils-image`) | `codex-rs/utils/image` | unit tests stripped from 1 file(s), 1 test file(s) deleted; manifest: removed `[dev-dependencies]`, `[[bench]]` |
| `crates/protocol` (`codex-protocol`) | `codex-rs/protocol` | unit tests stripped from 31 file(s); Windows-only code stripped from 1 file; manifest: removed `[dev-dependencies]`; dependency cuts below; new `src/network_proxy_types.rs` (copied from `codex-rs/network-proxy/src/{network_policy,config,environment_policy}.rs`, data types only) and `src/http_error.rs` (`RetryAfter` copied from `codex-rs/http-client/src/retry_after.rs`; `HttpError` new) |
| `crates/utils/output-truncation` (`codex-utils-output-truncation`) | `codex-rs/utils/output-truncation` | unit tests stripped from 1 file(s), 1 test file(s) deleted; manifest: removed `[dev-dependencies]` |
| `crates/file-system` (`codex-file-system`) | `codex-rs/file-system` | `lib.rs` re-exports the new `local` module; manifest gains the local implementation's dependencies (`tokio`, `tokio-util`, `tracing`, unix `libc`, `rustix`; all in Codex's lock) |
| `crates/file-system/src/local/{mod,read,regular_file}.rs`, `local/no_follow/{mod,unix}.rs` | `codex-rs/exec-server/src/{local_file_system,local_file_system_read,regular_file}.rs`, `no_follow/{mod,unix}.rs` | approved C2: unit tests and Windows code stripped; `SandboxedFileSystem` path and `ExecServerRuntimeOptions` removed (all operations run in-process); `FsOpenMode` copied from `codex-rs/exec-server-protocol/src/protocol.rs`; `open_file` made `pub` |
| `crates/shell-command` (`codex-shell-command`) | `codex-rs/shell-command` | unit tests stripped from 10 file(s), 2 test file(s) deleted; Windows-only code stripped from 1 file(s); deleted unreachable `src/command_safety/powershell_parser.rs`; manifest: removed `[dev-dependencies]` |
| `crates/history` (`codex-history`) | `codex-rs/history` | unit tests stripped from 5 file(s), 5 test file(s) deleted; manifest: removed `[dev-dependencies]` |
| `crates/context-fragments` (`codex-context-fragments`) | `codex-rs/context-fragments` | unit tests stripped from 2 file(s), 2 test file(s) deleted; manifest: removed `[dev-dependencies]` |
| `crates/git-utils` (`codex-git-utils`) | `codex-rs/git-utils` | unit tests stripped from 8 file(s), 4 test file(s) deleted; Windows-only code stripped from 3 file(s); manifest: removed `[dev-dependencies]` |
| `crates/apply-patch` (`codex-apply-patch`) | `codex-rs/apply-patch` | unit tests stripped from 6 file(s), 1 test file(s) deleted; manifest: removed `[dev-dependencies]`; `codex_exec_server::` paths replaced by `codex_file_system::` in `lib.rs`, `file_update.rs`, `invocation.rs`, `standalone_executable.rs` |
| `crates/codex-client` (`codex-client`) | `codex-rs/codex-client` | `codex-http-client` replaced; see cuts. New `src/request.rs` (copied from `codex-rs/http-client/src/request.rs`, unit tests stripped), `src/error.rs` and `src/transport.rs` (adapted from `codex-rs/http-client/src/{error,transport}.rs`) |
| `crates/model-provider-info` (`codex-model-provider-info`) | `codex-rs/model-provider-info` | unit tests stripped from 2 file(s), 2 test file(s) deleted; manifest: removed `[dev-dependencies]`; Amazon Bedrock/AWS and ChatGPT-backend support removed; `WireApi::ChatCompletions` (`wire_api = "chat_completions"`) added; `lib.rs` now 498 lines |
| `crates/sandboxing` (`codex-sandboxing`) | `codex-rs/sandboxing` | unit tests stripped from 9 file(s), 13 test file(s) deleted; Windows-only code stripped from 7 file(s); deleted unreachable `src/windows_mxc.rs`; manifest: removed `[target.'cfg(windows)'.dependencies]`, `[dev-dependencies]`; `src/windows.rs` deleted; managed network proxy, MXC and Windows sandbox removed from `manager.rs`, `seatbelt.rs`, `landlock.rs`, `violation.rs`, `spawn.rs`, `lib.rs` |
| `crates/linux-sandbox` (`codex-linux-sandbox`) | `codex-rs/linux-sandbox` | unit tests stripped from 12 file(s), 6 test file(s) deleted; manifest: removed `[target.'cfg(target_os = "linux")'.dev-dependencies]`; `src/proxy_routing.rs` and `src/proxy_lifecycle.rs` deleted; managed-network proxy removed from `linux_run_main.rs`, `seccomp.rs`, `bwrap.rs`, `lib.rs`; `bundled_bwrap.rs` no longer uses install-context. Linux-only code: compile check pending the Linux target |
| `crates/tools` (`codex-tools`) | `codex-rs/tools` | unit tests stripped from 18 file(s), 17 test file(s) deleted; manifest: removed `[dev-dependencies]`; `src/code_mode.rs` and `src/request_plugin_install.rs` deleted; `tool_discovery.rs` reduced to tool-search items; code-mode methods removed from `indirect_namespace_prefixes.rs`; `request_user_input_available_modes` takes a bool |
| `crates/rollout` (`codex-rollout`) | `codex-rs/rollout` | unit tests stripped from 14 file(s), 14 test file(s) deleted; manifest: removed `[dev-dependencies]`; `src/state_db.rs` and `src/sqlite_metrics.rs` deleted; `metadata.rs` reduced to `rollout_id_from_path` and `forked_from_ordinal_exclusive`; listing/lookup in `recorder.rs`, `list.rs`, `session_index.rs` are filesystem-only; new `src/otel_stub.rs` |
| `crates/llm` (`ether-llm`, new) | – | new crate: model-client interface (`client.rs`) and replay source (`replay.rs`) are new |
| `crates/llm/src/common.rs` | `codex-rs/codex-api/src/common.rs` | HTTP Responses request and stream event types only; access programs, memories, WebSocket requests, safety buffering, model verification, moderation, rate limits and models etag removed; `instructions` field added |
| `crates/llm/src/error.rs` | `codex-rs/codex-api/src/error.rs` | `RetryAfter` from codex-protocol; rate-limit header error removed |
| `crates/llm/src/sse/{mod,responses,responses_error}.rs` | `codex-rs/codex-api/src/sse/{mod,responses,responses_error}.rs` | unit tests removed; rate-limit, server-model, turn-state, safety-buffering, verification, moderation and telemetry handling removed |
| `crates/llm/src/responses.rs` | `codex-rs/codex-api/src/endpoint/{responses,session}.rs` | one `POST /responses` client with bearer API-key auth and `codex-client` retries; WebSocket, session headers, compression, turn state and telemetry removed |
| `crates/llm/src/responses_headers.rs` | `codex-rs/codex-api/src/responses_headers.rs` | none |
| `crates/engine` (`ether-engine`, new) | – | new crate: `engine.rs` (handle and submission loop), `session.rs`, `turn.rs` (turn loop shaped like `codex-rs/core/src/session/turn.rs` and `responses_retry.rs`), `sampling.rs` (stream handling from `session/turn.rs` and `stream_events_utils.rs` as a pure step), `config.rs`, `model_profile.rs` (defaults from `models-manager/src/model_info.rs`) and `tools.rs` are new |
| `crates/engine/prompt.md` | `codex-rs/models-manager/prompt.md` | none |
| `crates/engine/src/normalize.rs` | `codex-rs/core/src/context_manager/normalize.rs` | `ensure_call_outputs_present` and `remove_orphan_outputs` only; `error_or_panic` becomes `warn!` |
| `crates/engine/src/history.rs` | `codex-rs/core/src/context_manager/history.rs` | recording with tool-output truncation, prompt normalization and token accounting only; `close_open_calls` added; `replace` for compacted history |
| `crates/engine/src/event_mapping.rs` | `codex-rs/core/src/event_mapping.rs` | user, assistant and reasoning items only; own contextual-marker list |
| `crates/engine/src/context.rs` | `codex-rs/core/src/context/turn_aborted.rs` | user-role interrupted marker only |
| `crates/cli` (`ether-cli`, binary `ether`, new) | – | new crate; `replay/hello.jsonl` is a sample replay file |
| `crates/engine/src/tools/mod.rs` | – | new: `ToolHandler` interface and router; parallel-safe calls share a read/write lock as in `codex-rs/core/src/tools/parallel.rs` |
| `crates/engine/src/tools/plan.rs` | `codex-rs/core/src/tools/handlers/{plan,plan_spec}.rs` | on ether's `ToolHandler`; Plan-mode check removed |
| `crates/engine/src/rollout.rs` | – | new: opens the `codex-rollout` recorder for a session, applies its persistence policy, builds `TurnContextItem`s and finds sessions to resume; wiring shaped like `codex-rs/core/src/session/mod.rs` `persist_rollout_items` |
| `crates/engine/src/reconstruct.rs` | `codex-rs/core/src/session/rollout_reconstruction.rs` | forward replay from the newest compaction with replacement history, plus last token count and turn number; rollback, retained/guardian context, world state, windows and turn settings removed |
| `crates/engine/src/tools/apply_patch.rs` | `codex-rs/core/src/tools/handlers/{apply_patch,apply_patch_spec}.rs`, `core/src/apply_patch.rs`, `core/src/tools/runtimes/apply_patch.rs` | on ether's `ToolHandler`; new JSON function variant (`{"input": ...}`) beside the freeform one; approval and application inline (no orchestrator); applied in-process on `LOCAL_FS` without a sandboxed helper; no hooks, streaming argument diffs, environment selection, permission widening or telemetry; `intercept_apply_patch` kept for the exec handler |
| `crates/engine/assets/apply_patch.lark` | `codex-rs/core/assets/tools/apply_patch.lark` | provenance comment added |
| `crates/engine/src/safety.rs` | `codex-rs/core/src/safety.rs` | patch checks only; permission profile from `SandboxMode`; no executor-managed route, Windows level or platform-sandbox probe |
| `crates/engine/src/turn_diff_tracker.rs` | `codex-rs/core/src/turn_diff_tracker.rs` | one environment (no environment ids, one display root); unit tests removed; new `TurnDiffs` per-turn holder |
| `crates/engine/src/tools/view_image.rs` | `codex-rs/core/src/tools/handlers/{view_image,view_image_spec}.rs` | reads `LOCAL_FS`; resizes with `codex-utils-image` in the handler; `high` detail only; emits `ViewImageToolCall`; no modality check |
| `crates/engine/src/tools/request_user_input.rs` | `codex-rs/core/src/tools/handlers/{request_user_input,request_user_input_spec}.rs` | no collaboration-mode or sub-agent checks; always blocking; no guardian retained context |
| `crates/cli/replay/patch.jsonl` | – | new: replay where the model adds a file with `apply_patch` |
| `crates/engine/src/world_state/mod.rs` | `codex-rs/core/src/context/world_state/mod.rs`, `core/src/context_manager/updates.rs` | three typed sections compared by value; no extensions, placement, hashing, merge patches or legacy matching |
| `crates/engine/src/world_state/environment.rs` | `codex-rs/core/src/context/world_state/environment.rs`, `core/src/context/environment_context.rs` | one local environment: cwd, shell, current date |
| `crates/engine/src/world_state/permissions.rs` | `codex-rs/core/src/context/world_state/permissions.rs`, `codex-rs/prompts/src/permissions_instructions.rs` | bundled sandbox/approval templates only; no exec-policy prefixes, granular/auto-review details or denied reads |
| `crates/engine/src/world_state/agents_md.rs` | `codex-rs/core/src/context/world_state/agents_md.rs`, `core/src/context/user_instructions.rs` | typed snapshot only |
| `crates/engine/src/agents_md.rs` | `codex-rs/core/src/agents_md.rs` | local sync discovery from the git root to cwd plus `$ETHER_HOME/AGENTS.md` |
| `crates/engine/src/compact.rs` | `codex-rs/core/src/compact.rs`, `core/src/context/compaction_summary.rs`, `codex-rs/prompts/src/compact.rs` | local compaction only |
| `crates/engine/templates/permissions/*.md` | `codex-rs/prompts/templates/permissions/{sandbox_mode,approval_policy}/*.md` | none (flattened into one directory; `on_request_rule_request_permission.md` not copied) |
| `crates/engine/templates/compact/{prompt,summary_prefix}.md` | `codex-rs/prompts/templates/compact/*.md` | none |
| `crates/engine/src/exec/mod.rs` | `codex-rs/core/src/unified_exec/mod.rs`, `codex-rs/core/src/tools/context.rs` (`ExecCommandToolOutput`) | local processes only; constants and the model-facing output format kept |
| `crates/engine/src/exec/backend.rs` | – | new: C4 local process backend on `codex-utils-pty` (spawn via `codex-sandboxing` transform + `spawn_process`, stdin, output buffers, interrupt, kill), in the role of exec-server's local backend and `codex-rs/core/src/unified_exec/process.rs`; `collect_until` ports `collect_output_until_deadline` without the pause state |
| `crates/engine/src/exec/buffer.rs` | `codex-rs/core/src/unified_exec/head_tail_buffer.rs` | test helpers removed |
| `crates/engine/src/exec/manager.rs` | `codex-rs/core/src/unified_exec/process_manager.rs`, `tools/orchestrator.rs`, `tools/runtimes/unified_exec.rs`, `unified_exec/async_watcher.rs` | one local path; orchestrator sequence (approve → sandbox → attempt → retry) inlined; no exec-server, guardian, network approval, stdin approval, hooks, telemetry, snapshots |
| `crates/engine/src/exec/approval.rs` | `codex-rs/core/src/tools/sandboxing.rs`, `tools/approvals.rs`, `command_canonicalization.rs` | exec approval primitives only; approvals through `Session::request_approval`; no PowerShell |
| `crates/engine/src/exec/policy.rs` | `codex-rs/core/src/exec_policy.rs` | rules from `$ETHER_HOME/rules/*.rules` only; no amendments; POSIX only |
| `crates/engine/src/exec/shell.rs` | `codex-rs/core/src/shell.rs` | POSIX shells only; no exec-server shell info or PATH prepends |
| `crates/engine/src/tools/shell_spec.rs` | `codex-rs/core/src/tools/handlers/shell_spec.rs` | `exec_command`/`write_stdin` specs only, without login, environment_id, prefix_rule, additional_permissions |
| `crates/engine/src/tools/unified_exec.rs` | `codex-rs/core/src/tools/handlers/unified_exec.rs`, `unified_exec/{exec_command,write_stdin}.rs` | handlers on ether's `ToolHandler`; local only |
| `crates/cli/replay/exec.jsonl` | – | new: sample replay where the model runs `echo hi` through `exec_command` |
| `crates/llm/src/chat.rs` | – | new: Chat Completions request mapping and stream parser |
| `scripts/check-prefix.py` | – | new: prompt-cache prefix check over a request log, after the idea of `codex-rs/core/tests/suite/prompt_caching.rs` |
| `crates/tui` (`ether-tui`, new) | `codex-rs/tui/src` (lifted modules listed in `docs/PHASE5.md`) | lifted files are copied with unit tests and Windows code removed and keep their "Modified from OpenAI Codex" notes where changed; `app.rs` and `tui.rs` are new; local copies: `keymap_config.rs` (`codex-rs/config/src/tui_keymap.rs`), `rendering_config.rs` (`codex-rs/config/src/tui_rendering.rs`), `terminal_detection.rs` (`codex-rs/terminal-detection/src/lib.rs`), `elapsed.rs` (`codex-rs/utils/elapsed/src/lib.rs`); stand-ins: `clipboard_copy.rs`, `inline_visualization.rs`; `history_cell/mod.rs` and `notices.rs` reduced to ether's cells |
| `crates/tui/assets/themes/*.tmTheme` | `codex-rs/tui/assets/themes` | none |
| `crates/cli/replay/tui.jsonl` | – | new: replay file for the TUI tour |
| `crates/ui-spec` (`ether-ui-spec`, new) | – | new: generative UI `Node` types, parsing and limits (Phase 6) |
| `crates/engine/src/tools/render_ui.rs` | – | new: `render_ui` tool, shaped like `codex-rs/core/src/tools/handlers/plan.rs` |
| `crates/tui/src/ui_spec_cell.rs` | – | new: `UiSpecCell`, drawing views with ratatui widgets into an off-screen buffer |
| `crates/protocol/src/protocol.rs`, `crates/rollout/src/policy.rs` | `codex-rs/protocol/src/protocol.rs`, `codex-rs/rollout/src/policy.rs` | `EventMsg::UiRender` / `UiRenderEvent` added; transient like `PlanUpdate` (not written to the paginated rollout), so `render_ui` views do not come back on resume |
| `crates/cli/replay/ui.jsonl` | – | new: replay with an invalid `render_ui` spec, the fix, and an answer |
| `crates/tui/src/html_image.rs`, `html_fence.rs` | – | new: HTML views (Chrome rendering, kitty placeholders, stream fence filter, view instructions) |
| `crates/tui/src/kitty_diacritics.rs` | kitty's `rowcolumn-diacritics.txt` (kitty 0.49.2 docs) | the list of code points, as a Rust array |
| `crates/engine/src/world_state/display.rs` | – | new: `<terminal_display>` section, after the other world-state sections |
| `crates/cli/replay/html.jsonl`, `demo.jsonl` | – | new: replays with a ```` ```view ```` answer |
| `crates/cli/replay/architecture.jsonl` | – | new: GLM 5.3's live ```` ```view ```` answer drawing ether's architecture, replayed |
| `crates/tui/src/devtools.rs` | – | new: minimal Chrome DevTools Protocol client over `--remote-debugging-pipe` |
| `crates/tui/src/view_renderer.rs` | – | new: warm view renderer thread (page setup, kit runtime, scaled screenshot) |
| `crates/tui/src/view_kit/runtime.js`, `kit.css` | – | new: the view kit (components, connectors, checks, terminal-themed stylesheet) |
| `crates/cli/replay/kit.jsonl` | – | new: GLM 5.3's view written with the view kit, replayed |
| `crates/tui/src/view_kit/vendor/mermaid.min.js`, `mermaid.LICENSE` | npm `mermaid@11.17.2` `dist/mermaid.min.js` and `LICENSE` (MIT, Copyright (c) 2014 - 2022 Knut Sveidqvist) | unchanged; tarball sha1 `e3caf3717582c0e44e5d04cff043e025caec7cba` (matches the registry), file sha256 `581ed7d74bd9048d0e3a91363927d72ef22942d7722546b27f7cc29e35390eb8`; compiled into `ether-tui`, so binaries must ship the licence |
| `crates/cli/replay/mermaid.jsonl` | – | new: replay with Mermaid diagrams, view kit tiles and table, and a broken diagram |
| `crates/tui/src/prompt_history.rs`, `crates/tui/src/launch.rs` | – | new: prompt history; the TUI's launch options and session host trait |
| `crates/cli/replay/gate.jsonl` | – | new: Phase 3 gate replay (apply_patch adds a file, exec_command reads it back, final answer) |
| `THIRD_PARTY_NOTICES.md` | – | new: notices for the vendored Mermaid bundle (and the libraries inside it) and kitty's diacritics list |
| `crates/llm/src/overflow.rs` | prime-agent `crates/pa-ai/src/utils_inner/overflow.rs` (MIT) | the provider wordings and the rate-limit exclusions on `regex-lite`; Z.AI's `model_context_window_exceeded` and a generic "maximum context" added |
| `crates/tui/src/clipboard.rs` | prime-agent `crates/pa-tui/src/clipboard.rs`, `osc52.rs` (MIT) | no Windows; the OSC 52 bytes go back to the app, which writes them with the next draw |
| `crates/tui/src/insert_history.rs` (OSC 133 marks only) | prime-agent `crates/pa-tui/src/osc133.rs` (MIT) | the marks are written around an inserted batch instead of riding in spans |
