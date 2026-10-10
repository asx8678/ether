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
