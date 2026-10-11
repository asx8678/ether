# Status (2026-10-11)

Where ether stands, and what comes next. Details live in the phase docs and DECISIONS.md.

## Done

- Phases 0–6 of the brief (inventory, tier 1–2 crates, engine, GLM via Chat Completions, TUI, generative UI), each gate passed.
- Phase 6 follow-up: HTML views (```` ```view ````) and Mermaid diagrams (```` ```mermaid ````) drawn by a warm headless Chrome and shown with kitty image placeholders; the view kit (`x-bars`, `x-gauge`, `x-spark`, `x-stat`, `x-table`, cards, badges).
- Phase 7 (daily driver): prompt history, resume shows the conversation, slash commands, `@` file mentions, release binary installed at `~/.cargo/bin/ether`.
- Full review (2026-10-11) and fixes in three batches: untrusted project config limited to safe keys, approval queue, turn abort/shutdown ordering, chat retry/truncation, 429 and overflow handling, TUI ordering and layout, plain-mode exit status, third-party notices, doc drift, Windows leftovers.
- Phase 8 (quick wins from prime-agent): wider overflow detection, `prompt_cache_key`, quota parking and jitter, `/copy`, OSC 133 prompt marks, exit robustness. See PHASE8.md.
- Pushed to `origin/main` (github.com/asx8678/ether) up to the review fixes; the status note and Phase 8 are local commits, not pushed yet.

## Open decisions (waiting on you)

- `path-uri`'s Windows path handling (100+ call sites): remove or keep.
- `request_user_input`: build a question prompt in the TUI so the tool can be offered again.

## Next

1. Medium (prime-agent designs): structured incremental compaction summaries, image paste, external editor and Ctrl-Z, per-provider compat flags.
2. Multi-agent phase (planning round first): borrow prime-agent's designs, not its code: the subagent host contract (spawn/list/collect/delete/rename, `pa-core/src/session_engine/rlm_host.rs`), family-scoped agent messaging at turn boundaries, a goal driver with a persisted no-progress cap, autonomous mode as an outside policy, a cron/heartbeat scheduler, an append-only spawn ledger. Avoid process-per-session at first; `ractor` is not in Codex's lock (tokio tasks or Elixir instead).
3. Later: durable memory delivered only at cold boundaries, MCP with three fixed meta-tools, web search.

## Working notes

- Check: `~/.cache/ether/tools/chk.sh` (re-seeds `Cargo.lock` from Codex, runs `cargo check`); clippy and `cargo fmt --all -- --check` should stay clean.
- Smoke runs: the replays in `crates/cli/replay/` in plain mode and the TUI (`~/.cache/ether/tools/tui_drive.py` drives a pseudo-terminal); `scripts/check-prefix.py` on `--request-log` output.
- Codex reference export: `~/.cache/ether/codex-806d973/codex-rs` (keep it for upstream updates).
- Update the installed binary: `cargo build --release -p ether-cli && install -m 755 target/release/ether ~/.cargo/bin/ether`.
