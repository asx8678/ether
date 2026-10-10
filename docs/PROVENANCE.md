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
