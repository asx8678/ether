# Phase 5: TUI (option B)

A slim new TUI, `ether-tui`, built from Codex's rendering code (`codex-rs/tui/src` @ 806d973). Its app shell, chat widget and composer are new and drive `ether_engine::Engine` directly through `Op` in and `EventMsg` out. The TUI is the default `ether` interface on a terminal; the plain-text mode stays for pipes and `--replay` without a terminal.

## Lifted modules

About 23k non-test lines of the TUI's rendering code, with these cuts:

- `keymap` (2.7k lines, needs `codex-config`): replaced by a small fixed-keybinding module; no Vim mode, no configurable keys.
- `codex-terminal-detection`: the few functions used are copied locally instead of copying the crate.
- Terminal probing, desktop notifications, clipboard integration and Codex's `tui.rs` terminal setup: not copied; the app shell sets up the terminal itself.

## Work

- [ ] Terminal and rendering base: `custom_terminal`, `insert_history`, `wrapping`, `render/`, `width`, `line_truncation`, `terminal_hyperlinks`, `style`, `color`, `terminal_palette`.
- [ ] Markdown: `markdown`, `markdown_render`, `markdown_stream`, `markdown_text_merge`, `markdown_copy`, `table_detect`, `streaming/`, `render/highlight`.
- [ ] Diffs: `diff_render`, `diff_model`.
- [ ] Input: `bottom_pane/textarea`, `bottom_pane/paste_burst`, `key_hint`.
- [ ] Cells: user message, agent message (streamed markdown), reasoning, command with output and exit status, patch with diff, plan update, errors and warnings (from `history_cell/` and `exec_cell/`, with ether's `EventMsg`/`TurnItem` in place of app-server types).
- [ ] App shell: event loop over terminal input and engine events; history inserted into the terminal scrollback (inline mode) above a bottom pane.
- [ ] Composer: Enter sends, Shift+Enter newline, Esc or Ctrl-C interrupts a running turn, Ctrl-D exits.
- [ ] Approval overlay for commands and patches: yes, yes for this session, no.
- [ ] Status line: model, working directory, token usage with cached tokens.
- [ ] `ether` uses the TUI on a terminal; `--resume`/`--last` work as in plain mode.

## Not in this phase

`@` file search, slash commands, a transcript view, a resume picker and up-arrow prompt history (which needs Codex's `message-history` crate).

## Gate

Run the TUI live against GLM and with `--replay`: streamed markdown, a command with output, a diff for a patch, the approval overlay, an interrupt, and cached tokens in the status line. The brief's insta snapshot tests are dropped (no tests).
