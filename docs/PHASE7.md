# Phase 7: daily driver

The items Phase 5 left out, so ether is comfortable to use every day. All in `ether-tui` and `ether-cli`, plus one engine change for resume.

## Work

- [x] Prompt history: Up and Down recall earlier prompts, across sessions; saved to `$ETHER_HOME/history.jsonl` (mode 0600).
- [x] Resume shows the conversation: on `--resume` / `--last`, the engine replays the earlier user and assistant messages to the client as `UserMessage` / `AgentMessage` events, sent without being recorded again; the TUI draws them (views and diagrams included). Plain mode ignores them.
- [x] Slash commands: `/help`, `/status`, `/compact`, `/new`, `/resume` (a picker of this directory's sessions), `/quit`. The CLI gives the TUI a session host to start, resume and list sessions.
- [x] `@` file mentions: typing `@` and part of a path shows fuzzy matches from `codex-file-search` (gitignore-aware); Tab or Enter inserts the path.
- [x] A release build installed as `ether` on the PATH: `~/.cargo/bin/ether` (40 MB with line tables, as Codex's release profile). To update after changes: `cargo build --release -p ether-cli && install -m 755 target/release/ether ~/.cargo/bin/ether`.

## Not in this phase

`/model` (switching models inside a session needs engine support), a transcript view, the multi-agent layer.

## Gate

- [x] Each item checked by driving `ether` in a pseudo-terminal (2026-10-10): prompts recalled with Up across sessions (the history file is created with mode 0600 even before `$ETHER_HOME` exists); `/help` and `/status`; an `@prompt_hi` mention completed to `crates/tui/src/prompt_history.rs` with Tab; `/resume` listing the directory's sessions and resuming one with its conversation; `/new`; resuming a session whose answer had a view draws the view again.
- [x] Live with GLM 5.3: a prompt with an `@` mention completed by Tab, GLM read the file and answered, `/status` showed cached tokens; `ether --last` showed the earlier conversation and Up recalled the last prompt.
