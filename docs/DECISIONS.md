# Decisions

One line per decision, with the reason. Newest last.

- Pin Codex at `806d973` (your checkout's HEAD, `c3d3b14` plus one code-mode-only commit): the verified facts still hold and provenance matches the checkout.
- Copy from a `git archive` export in `~/.cache/ether/codex-806d973`: `CODEX_SRC` is never written and untracked files never leak in.
- Target is `ether` (workspace `ether`); new crates will be `ether-engine`, `ether-llm`, `ether-tui`, `ether-cli`. Copied crates keep their `codex-*` names until a rename you approve.
- Crates already in Codex's `Cargo.lock` count as approved dependencies; anything else needs your approval.
- `Cargo.lock` is re-seeded from Codex's lock before every `cargo check`: every dependency resolves to the exact version Codex ships.
- Toolchain 1.95.0, edition 2024, Codex `rustfmt.toml`, `clippy.toml` (minus the sqlx rules) and `[workspace.lints]`: copied code builds and lints as it does upstream.
- Only the `dev` and `release` profiles are kept: `dev-small`, `profiling` and `ci-test` serve Codex CI.
- No tests (your instruction): nothing is written, copied or run. Copied crates lose `#[cfg(test)]` items, `*_tests.rs` files, `tests/`, `benches/`, insta `snapshots/` and dev-dependencies. The check is building and running the binary.
- Linux and macOS only: Windows-only code (`cfg(windows)`, `cfg(target_os = "windows")`, `cfg(all(windows, ...))`) and Windows-only dependency tables are stripped from copied crates. Linux gets checked later on a Linux machine.
- `BUILD.bazel` files are not copied: ether builds with Cargo only.
- The mechanical cuts are scripted (`scripts/strip-cfg.pl`, `scripts/orphans.pl`, `scripts/toml-strip.pl`) so they can be reapplied to a newer Codex.
- Layout is `crates/<codex dir>`. `ext/items` becomes `crates/extension-items`; `vendor/bubblewrap` becomes `crates/vendor/bubblewrap`, so `bwrap/build.rs` is unchanged.
- `vendor/bubblewrap` keeps its own LGPL-2.0-or-later licence (`COPYING`); its `tests/` are not copied.
- C1: keep `codex-extension-items` (used by protocol, tools and rollout; 172 lines).
- Sandboxing has no managed network proxy: network access is on or off per the permission profile, enforced by Seatbelt (macOS) and the Linux helper. Reason: network-proxy is on the never-copy list and proxy filtering is out of scope.
- The realtime/guardian/cyber-access protocol types stay until the Phase 3 engine port, which rewrites `Op`/`EventMsg` handling anyway (cut was over the 30-site limit).
- Linux-only crates are type-checked from macOS with `scripts/check-linux.sh` (`aarch64-unknown-linux-gnu` target, bubblewrap C build skipped, blake3 in pure-Rust mode). Crates that compile C for the target (zstd, tree-sitter) wait for a real Linux machine.
- File rewrites that add notes go through `scripts/note.pl`, never through the 50 KB-capped tool read (two files were truncated once and rebuilt from source).
- The new wire API is spelled `wire_api = "chat_completions"` (`WireApi::ChatCompletions`); plain `"chat"` stays rejected as an unknown variant, so old Codex configs fail loudly rather than silently changing meaning.
- C2 detail: the ported `LocalFileSystem` has no sandboxed helper process; filesystem calls run in-process and write permission is checked by the engine before a patch is applied (as Codex core already does).
- C2: copy exec-server's `local_file_system.rs` and `local_file_system_read.rs` into `codex-file-system` as its local implementation.
- C3: drop rollout's SQLite state (`state_db.rs`, `sqlite_metrics.rs`, SQLite-only parts of `metadata.rs`); listing and replay use JSONL files only.
- C5: carry only the crossterm `[patch.crates-io]` fork; the tungstenite patches are dropped because no websocket code is copied.
- C6: drop dev-dependencies on crates not on the list. All dev-dependencies are removed anyway now that tests are gone.
- Phase 3 engine is new code shaped like Codex core, lifting functions and match arms where they stand alone: core's files reach into `Session`, config, guardian, hooks, analytics and exec-server almost everywhere, so they cannot be copied whole. The to-do list is `docs/PHASE3.md`.
- C4: the process backend is a small local one on `codex-utils-pty` (spawn, stdin, output, kill), with the `codex-sandboxing` transform applied before spawn; exec-server's local backend (~4.5k lines) is not copied and shell snapshots are left out.
- Config lives in `~/.ether/config.toml`, with an optional project `.ether/config.toml` and CLI overrides on top; `codex-config` is not copied.
- The Phase 3 runnable is a plain-text `ether` command (prompt in, streamed answer and tool activity out); the Phase 5 TUI replaces it.
- `ether --replay FILE` feeds recorded Responses SSE through the real parser in place of a network model. It is a runtime feature for running the binary before Phase 4 connects GLM, not a test.
