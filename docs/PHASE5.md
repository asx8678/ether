# Phase 5: TUI (option B)

A slim new TUI, `ether-tui`, built from Codex's rendering code (`codex-rs/tui/src` @ 806d973). Its app shell, chat widget and composer are new and drive `ether_engine::Engine` directly through `Op` in and `EventMsg` out. The TUI is the default `ether` interface on a terminal; the plain-text mode stays for pipes and `--replay` without a terminal.

## Lifted modules

About 23k non-test lines of the TUI's rendering code, with these cuts:

- `keymap`: kept with its default bindings only (the textarea is built on it); its config types are a local copy of `codex-rs/config/src/tui_keymap.rs`. No user keymap config.
- `codex-terminal-detection`: the few functions used are copied locally instead of copying the crate.
- Desktop notifications, clipboard integration (stand-in types only), inline visualizations (stand-in), Mermaid rendering (fences render as source) and Codex's `tui.rs` terminal setup: not copied; ether's `tui.rs` sets up the terminal, lifting the viewport-growth logic (`tui/scrollback.rs`).

## Work

- [x] Terminal and rendering base: `custom_terminal`, `insert_history`, `wrapping`, `render/`, `width`, `line_truncation`, `terminal_hyperlinks`, `style`, `color`, `terminal_palette`.
- [x] Markdown: `markdown`, `markdown_render`, `markdown_stream`, `markdown_text_merge`, `markdown_copy`, `table_detect`, `streaming/`, `render/highlight`.
- [x] Diffs: `diff_render`, `diff_model`.
- [x] Input: `bottom_pane/textarea`, `key_hint` (bracketed paste replaces `paste_burst`).
- [x] Cells: user message, agent message (streamed markdown), reasoning, command with output and exit status, patch with diff, plan update, errors and warnings (from `history_cell/` and `exec_cell/`, with ether's `EventMsg`/`TurnItem` in place of app-server types).
- [x] App shell: event loop over terminal input and engine events; history inserted into the terminal scrollback (inline mode) above a bottom pane.
- [x] Composer: Enter sends, Shift+Enter newline, Esc or Ctrl-C interrupts a running turn, Ctrl-D exits.
- [x] Approval overlay for commands and patches: yes, yes for this session, no.
- [x] Status line: model, working directory, token usage with cached tokens.
- [x] `ether` uses the TUI on a terminal; `--resume`/`--last` work as in plain mode.

## Not in this phase

`@` file search, slash commands, a transcript view, a resume picker and up-arrow prompt history (which needs Codex's `message-history` crate).

## Gate

- [x] Run the TUI live against GLM and with `--replay`: streamed markdown, a command with output, a diff for a patch, the approval overlay, an interrupt, and cached tokens in the status line. The brief's insta snapshot tests are dropped (no tests). Checked 2026-10-10 by driving `ether` in a pseudo-terminal and reading the emulated screen: `crates/cli/replay/tui.jsonl` (markdown, command output with exit code, interrupt), `gate.jsonl` with `--approval untrusted` (patch and command approval overlays, diff), and a live GLM 5.3 turn (ls, apply_patch diff, markdown summary, `6441 in (6400 cached)`), then `--last` resuming it in the TUI.
