#!/bin/sh
# Type-check the Linux-only sandbox crates from macOS (no Linux C toolchain needed).
# CODEX_SKIP_BWRAP_BUILD skips compiling vendored bubblewrap; CARGO_FEATURE_PURE makes blake3
# (via execpolicy -> starlark) use its pure-Rust backend instead of NEON C code.
# Crates that build C for the target (zstd, tree-sitter) are not included.
set -e
rustup target add aarch64-unknown-linux-gnu >/dev/null
CARGO_FEATURE_PURE=1 CODEX_SKIP_BWRAP_BUILD=1 cargo check --target aarch64-unknown-linux-gnu \
  -p codex-linux-sandbox -p codex-sandboxing -p codex-uds -p codex-utils-pty \
  -p codex-process-hardening -p codex-bwrap -p codex-file-system "$@"
