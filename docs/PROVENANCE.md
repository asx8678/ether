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
