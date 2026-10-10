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
| `crates/execpolicy` (`codex-execpolicy`) | `codex-rs/execpolicy` | unit tests stripped from 2 file(s), 1 test file(s) deleted; Windows-only code stripped from 1 file(s); manifest: removed `[dev-dependencies]` |
| `crates/utils/image` (`codex-utils-image`) | `codex-rs/utils/image` | unit tests stripped from 1 file(s), 1 test file(s) deleted; manifest: removed `[dev-dependencies]`, `[[bench]]` |
| `crates/protocol` (`codex-protocol`) | `codex-rs/protocol` | unit tests stripped from 31 file(s); Windows-only code stripped from        1 file(s); manifest: removed `[dev-dependencies]`; dependency cuts below; new `src/network_proxy_types.rs` (copied from `codex-rs/network-proxy/src/{network_policy,config,environment_policy}.rs`, data types only) and `src/http_error.rs` (`RetryAfter` copied from `codex-rs/http-client/src/retry_after.rs`; `HttpError` new) |
| `crates/utils/output-truncation` (`codex-utils-output-truncation`) | `codex-rs/utils/output-truncation` | unit tests stripped from 1 file(s), 1 test file(s) deleted; manifest: removed `[dev-dependencies]` |
| `crates/file-system` (`codex-file-system`) | `codex-rs/file-system` | `lib.rs` re-exports the new `local` module; manifest gains the local implementation's dependencies (`tokio`, `tokio-util`, `tracing`, unix `libc`, `rustix`; all in Codex's lock) |
| `crates/file-system/src/local/{mod,read,regular_file}.rs`, `local/no_follow/{mod,unix}.rs` | `codex-rs/exec-server/src/{local_file_system,local_file_system_read,regular_file}.rs`, `no_follow/{mod,unix}.rs` | approved C2: unit tests and Windows code stripped; `SandboxedFileSystem` path and `ExecServerRuntimeOptions` removed (all operations run in-process); `FsOpenMode` copied from `codex-rs/exec-server-protocol/src/protocol.rs`; `open_file` made `pub` |
| `crates/shell-command` (`codex-shell-command`) | `codex-rs/shell-command` | unit tests stripped from 10 file(s), 2 test file(s) deleted; Windows-only code stripped from 1 file(s); deleted unreachable `src/command_safety/powershell_parser.rs`; manifest: removed `[dev-dependencies]` |
| `crates/history` (`codex-history`) | `codex-rs/history` | unit tests stripped from 5 file(s), 5 test file(s) deleted; manifest: removed `[dev-dependencies]` |
| `crates/context-fragments` (`codex-context-fragments`) | `codex-rs/context-fragments` | unit tests stripped from 2 file(s), 2 test file(s) deleted; manifest: removed `[dev-dependencies]` |
| `crates/git-utils` (`codex-git-utils`) | `codex-rs/git-utils` | unit tests stripped from 8 file(s), 4 test file(s) deleted; Windows-only code stripped from 3 file(s); manifest: removed `[dev-dependencies]` |
