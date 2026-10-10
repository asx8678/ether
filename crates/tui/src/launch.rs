// New in ether: what the TUI needs from its host (the CLI).
//! The TUI drives an [`Engine`] through `Op` / `EventMsg` only. To start another session
//! (`/new`, `/resume`) it asks the host, which owns the configuration and the model client.

use ether_engine::Engine;
use futures::future::BoxFuture;
use std::path::PathBuf;
use std::sync::Arc;

/// Everything [`crate::run`] needs.
pub struct Launch {
    pub engine: Engine,
    /// Submitted as soon as the TUI starts.
    pub initial_prompt: Option<String>,
    /// `$ETHER_HOME`, where the prompt history is kept.
    pub ether_home: PathBuf,
    pub sessions: Arc<dyn SessionHost>,
}

/// Starts, resumes and lists sessions for the TUI.
pub trait SessionHost: Send + Sync {
    /// A fresh session with the same configuration.
    fn new_session(&self) -> BoxFuture<'static, anyhow::Result<Engine>>;
    /// Resumes the session recorded at `path`.
    fn resume_session(&self, path: PathBuf) -> BoxFuture<'static, anyhow::Result<Engine>>;
    /// Sessions recorded in the working directory, newest first.
    fn list_sessions(&self) -> BoxFuture<'static, anyhow::Result<Vec<SessionEntry>>>;
}

/// A recorded session in the `/resume` picker.
#[derive(Debug, Clone)]
pub struct SessionEntry {
    pub path: PathBuf,
    /// When it was last updated, for display (for example `2026-10-10 21:40`).
    pub updated: String,
    /// The first prompt or another short description.
    pub preview: String,
}
