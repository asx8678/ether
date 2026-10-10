# Cuts

Every dependency or module removed from a copied crate, and how it was removed.

These apply to every crate and are only listed per crate when something besides them was removed:

- `[dev-dependencies]`, `[[bench]]` and `[[test]]` targets are deleted from the manifest;
- `#[cfg(test)]` items are deleted, and the `*_tests.rs` files they referenced become unreachable and are deleted;
- `#[cfg(windows)]` items and `[target.'cfg(windows)'.*]` dependency tables are deleted, and modules that only Windows code reached are deleted.

| Crate | Removed | How |
|---|---|---|
| `codex-utils-absolute-path` | dev-dependencies: `pretty_assertions`, `serde_json`, `tempfile` | deleted from the manifest |
| `codex-utils-string` | dev-dependencies: `pretty_assertions` | deleted from the manifest |
| `codex-utils-stream-parser` | dev-dependencies: `pretty_assertions` | deleted from the manifest |
| `codex-utils-cache` | dev-dependencies: `tokio` | deleted from the manifest |
| `codex-utils-process` | dev-dependencies: `pretty_assertions`, `tokio` | deleted from the manifest |
| `codex-async-utils` | dev-dependencies: `pretty_assertions` | deleted from the manifest |
| `codex-ansi-escape` | dev-dependencies: `pretty_assertions`, `tracing-subscriber` | deleted from the manifest |
| `codex-file-search` | dev-dependencies: `pretty_assertions`, `tempfile` | deleted from the manifest |
| `codex-process-hardening` | dev-dependencies: `pretty_assertions` | deleted from the manifest |
| `codex-uds` | Windows dependencies: `async-io`, `tokio-util`, `uds_windows`, `windows-sys`; dev-dependencies: `pretty_assertions`, `tempfile`, `tokio`; modules: `src/windows_peer.rs`, `src/windows_security.rs`, `src/windows_socket_validation.rs` | deleted from the manifest; unreachable module files deleted |
| `codex-utils-pty` | dev-dependencies: `codex-utils-cargo-bin`, `ctor`, `tempfile`, `pretty_assertions`; Windows dependencies: `filedescriptor`, `lazy_static`, `log`, `shared_library`, `winapi`; modules: `src/win/conpty.rs`, `src/win/job.rs`, `src/win/mod.rs`, `src/win/procthreadattr.rs`, `src/win/psuedocon.rs`, `src/windows_input.rs` | deleted from the manifest; unreachable module files deleted |
| `codex-utils-path-uri` | dev-dependencies: `pretty_assertions`, `serde_json` | deleted from the manifest |
| `codex-utils-home-dir` | dev-dependencies: `pretty_assertions`, `tempfile` | deleted from the manifest |
| `codex-utils-path` | Windows dependencies: `windows-sys`; dev-dependencies: `pretty_assertions`, `tempfile` | deleted from the manifest |
| `codex-extension-items` | dev-dependencies: `pretty_assertions` | deleted from the manifest |
| `codex-execpolicy` | dev-dependencies: `pretty_assertions` | deleted from the manifest |
| `codex-utils-image` | dev-dependencies: `divan`, `image`; bench target `[[bench]]` (and `benches/`) | deleted from the manifest |
| `codex-protocol` | dev-dependencies: `anyhow`, `pretty_assertions`, `tempfile`, `tokio` | deleted from the manifest |
| `codex-protocol` | `codex-network-proxy` (3 refs in 2 files) | `NetworkPolicyDecision`, `NetworkDecisionSource`, `EnvironmentNetworkPolicy` and the permission maps it holds copied into `src/network_proxy_types.rs` without proxy runtime/config methods |
| `codex-protocol` | `codex-http-client` (2 refs in 1 file) | `RetryAfter` copied into `src/http_error.rs`; `HttpError` (was `RouteAwareRequestError`) replaced by a local error with message, optional status and source; added `httpdate` (in Codex lock) |
