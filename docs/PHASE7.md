# Phase 7: daily driver

The items Phase 5 left out, so ether is comfortable to use every day. All in `ether-tui` and `ether-cli`, plus one engine change for resume.

## Work

- [x] Prompt history: Up and Down recall earlier prompts, across sessions; saved to `$ETHER_HOME/history.jsonl` (mode 0600).
- [x] Resume shows the conversation: on `--resume` / `--last`, the engine replays the earlier user and assistant messages to the client as `UserMessage` / `AgentMessage` events, sent without being recorded again; the TUI draws them (views and diagrams included). Plain mode ignores them.
- [x] Slash commands: `/help`, `/status`, `/compact`, `/new`, `/resume` (a picker of this directory's sessions), `/quit`. The CLI gives the TUI a session host to start, resume and list sessions.
- [x] `@` file mentions: typing `@` and part of a path shows fuzzy matches from `codex-file-search` (gitignore-aware); Tab or Enter inserts the path.
- [ ] A release build installed as `ether` on the PATH.

## Not in this phase

`/model` (switching models inside a session needs engine support), a transcript view, the multi-agent layer.

## Gate

Each item checked by driving `ether` in a pseudo-terminal, and once live with GLM 5.3.
