# Phase 8: quick wins from prime-agent

Small improvements whose designs come from prime-agent (MIT, `/Users/adam2/projects/prime-agent`), reviewed 2026-10-11. Code adapted from it carries a "Modified from prime-agent" note and is listed in THIRD_PARTY_NOTICES.md and PROVENANCE.md.

## Work

- [x] Wider context-overflow detection (`crates/llm/src/overflow.rs`): provider wordings on any 4xx, Chat error chunks and `finish_reason`s (Z.AI's `model_context_window_exceeded`), and a Chat response that stops at `length` with nothing written. Each compacts once and retries.
- [x] `prompt_cache_key` = thread id on every request (Responses and Chat, compaction too).
- [x] Quota parking and jitter (`crates/engine/src/turn.rs`): a `Retry-After` over 60 s parks the turn until the reset (at most 15 min including jitter, 3 times, interruptible, "Rate limited; waiting until HH:MM:SS"); server-reported waits get up to 20% jitter.
- [x] `/copy` (`crates/tui/src/clipboard.rs`): the last answer to the clipboard through pbcopy / termux / wl-copy / xclip / xsel, tmux's buffer, or OSC 52.
- [x] OSC 133 marks around the user's prompts in scrollback.
- [x] Exit robustness: terminal restored before the shutdown waits (colours, attributes, cursor too); SIGHUP and SIGTERM quit like `/quit`; the engine wait is capped at 20 s.

## Not in this phase

Session-affinity headers (Zro documents none), parking across process restarts (prime-agent's durable wake jobs), a double-Ctrl-C watchdog (with the terminal restored first, Ctrl-C already ends a slow exit). Compaction's own retry loop still uses plain backoff and does not park on a quota `Retry-After`.

## Gate

- [x] `cargo check`, `cargo clippy --workspace` and `cargo fmt --check` clean; all 11 replays pass in plain mode (2026-10-11).
- [x] Live with GLM 5.3 via Zro: two prompts in one session, both requests carried `prompt_cache_key`, Zro accepted it and the second hit 6,144 of 6,190 input tokens from cache.
- [x] In a pseudo-terminal: `/copy` before any answer warns; after the answer it puts exactly the answer text on the macOS clipboard through pbcopy ("copied the last answer"); with no clipboard helper on the PATH it falls back to OSC 52 carrying exactly the answer text; the prompt is wrapped in one OSC 133 `A` and one `B`+`C`; SIGTERM ends the TUI with status 0, the viewport cleared, bracketed paste off and the cursor shown.
- [ ] Not exercised: a real 429 with a long `Retry-After` (Zro gave none), tmux forwarding, Wayland/X11 helpers.
