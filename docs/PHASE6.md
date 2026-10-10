# Phase 6: generative UI, first slice

The model can draw a small terminal view with the built-in `render_ui` tool. The view is a tree of nodes, checked by the engine and drawn by the front-end. Nothing in this phase is copied from Codex.

## Work

- [x] `ether-ui-spec` (new crate, serde only): `Node` (`tag = "type"`, snake_case) with `column`, `row`, `text`, `markdown`, `table` and `gauge`; `parse` turns the model's JSON into a `Node`, checks limits (depth 8, 200 nodes, 12 columns, 200 rows, one cell per header, a positive gauge `max`) and words errors for the model with the failing node's path (`spec.children[0].rows[0]: 2 cells but 3 headers; ...`).
- [x] `render_ui` tool (`crates/engine/src/tools/render_ui.rs`): arguments `{spec, view_id?}`; returns only `ok view_id=...`; parse errors go back as the tool result ("invalid spec, nothing was drawn ... Fix the spec and call render_ui again."). Unnamed views are numbered per session (`view-1`, ...). The spec reaches the client as `EventMsg::UiRender { call_id, view_id, spec }`. On by default; `[ui] render_tool = false` in a config file removes it.
- [x] `UiSpecCell` (`crates/tui/src/ui_spec_cell.rs`): draws ratatui widgets into an off-screen `Buffer` sized to fit (`Table` with a rounded border and title, `LineGauge`, wrapped `Paragraph`s, the lifted markdown renderer; `row` splits the width equally) and converts each buffer row into a styled `Line`. Sibling gauges in a column line up their bars.
- [x] Live during the turn: a view stays in the viewport and a later call with the same `view_id` replaces it; it moves into history when anything is written after it (streamed text, a finished command, another cell) or when the turn ends, so history keeps the order things happened in.
- [x] Plain mode prints the same layout as text, 80 columns, without colours (`ether_tui::render_ui_plain`).
- [x] Replay: `crates/cli/replay/ui.jsonl` (an invalid spec, the fixed view, a final answer).

## Not in this phase

Interactive views (focus, input, buttons), views rebuilt on `--resume` (the TUI does not replay past history yet), and the brief's insta snapshots on `display_lines(80)` (no tests).

## Gate

- [x] GLM draws a table and a gauge on request, and fixes an invalid spec after the error comes back. Checked 2026-10-10 live with GLM 5.3: a planets table plus a `Disk 63%` gauge; then a deliberately invalid table (row with a missing cell), the error `spec.children[0].rows[0]: 2 cells but 3 headers` came back, and the next call fixed it. Five requests passed `scripts/check-prefix.py`, with cached tokens on every request after the first. Also checked in the TUI in a pseudo-terminal: live GLM turn with a row of a table and two gauges, and `ui.jsonl` by replay.

## Follow-up: HTML views

The model can also answer with HTML and CSS: a ```` ```view ```` block is rendered as an image in place of the code on terminals with kitty's Unicode-placeholder images (kitty, Ghostty) when Chrome or Chromium is installed.

- Rendering (`crates/tui/src/html_image.rs`): headless Chrome screenshots the page at the terminal's pixel width (transparent background, the terminal's text colour, font scaled to the cell size), the PNG is cropped to the cell grid and uploaded with kitty's graphics protocol, and the image is shown through `U+10EEEE` placeholder cells, so it is ordinary text in the scrollback.
- Safety: static only. The page's CSP (`default-src 'none'`) blocks scripts and all loads; Chrome runs with a throwaway profile, unresolvable host names and a dead proxy for all traffic (which also stops `<meta http-equiv="refresh">` navigation to an IP address; checked with a local listener).
- Streaming (`crates/tui/src/html_fence.rs`): view blocks are held out of the streamed answer, rendered as soon as they close, and text after them waits so the order is kept; a "rendering view…" line shows meanwhile; a failed render shows the source; an interrupt flushes what is held.
- Teaching: the CLI checks the capability before the engine starts and passes `view_instructions` (about 150 tokens: what renders, when to use a view, an example) as the `<terminal_display>` world-state section, fixed for the session. `render_ui` is then off unless `[ui] render_tool = true`. ```` ```html ```` stays code.
- Checked live with GLM 5.3 in a pseudo-terminal: views for number comparisons (crate sizes as bars, disk usage as gauges), prose for an explanation, code for "show me the HTML for a login form"; the prefix check passes. Replays: `crates/cli/replay/html.jsonl`, `demo.jsonl`.

Not yet: a text fallback for terminals without images, a persistent Chrome (each view starts one, about a second), interactive views.
