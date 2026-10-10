// New in ether: session recording through `codex-rollout`, a slim form of Codex's rollout wiring (codex-rs/core/src/session/mod.rs `persist_rollout_items` and the thread store @ 806d973).
//! Each session is written to `$ETHER_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl`, the layout of
//! Codex's `~/.codex/sessions`. Items pass through the rollout crate's persistence policy before
//! they are queued. Recording is best effort: failures are logged and the session goes on.

use crate::config::EngineConfig;
use codex_protocol::ThreadId;
use codex_protocol::config_types::SandboxMode;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::ThreadHistoryMode;
use codex_protocol::protocol::TurnContextItem;
use codex_rollout::RolloutConfig;
use codex_rollout::RolloutItem;
use codex_rollout::RolloutRecorder;
use codex_rollout::RolloutRecorderParams;
use codex_rollout::SortDirection;
use codex_rollout::ThreadSortKey;
use codex_rollout::persisted_rollout_items;
use std::path::Path;
use std::path::PathBuf;

const ORIGINATOR: &str = "ether";
/// Page size used when scanning for the newest session.
const LIST_PAGE_SIZE: usize = 25;

/// The open rollout file of one session.
pub(crate) struct Rollout {
    recorder: RolloutRecorder,
    history_mode: ThreadHistoryMode,
}

impl Rollout {
    /// Prepares a new rollout. The file is created by the first [`Rollout::persist`].
    ///
    /// New rollouts are paginated: user and agent messages are kept as `ItemCompleted` events,
    /// which ether emits, rather than the legacy `UserMessage` / `AgentMessage` events.
    pub(crate) async fn create(
        config: &EngineConfig,
        thread_id: ThreadId,
    ) -> std::io::Result<Self> {
        let history_mode = ThreadHistoryMode::Paginated;
        let params = RolloutRecorderParams::new(
            thread_id,
            /*forked_from_id*/ None,
            /*parent_thread_id*/ None,
            SessionSource::Cli,
            /*thread_source*/ None,
            ORIGINATOR.to_string(),
            BaseInstructions {
                text: config.base_instructions.clone(),
                provenance: None,
            },
            /*dynamic_tools*/ Vec::new(),
        )
        .with_history_mode(history_mode);
        let recorder = RolloutRecorder::new(&rollout_config(config), params).await?;
        Ok(Self {
            recorder,
            history_mode,
        })
    }

    /// Opens an existing rollout for appending.
    pub(crate) async fn resume(
        config: &EngineConfig,
        path: PathBuf,
        history_mode: ThreadHistoryMode,
    ) -> std::io::Result<Self> {
        let params = RolloutRecorderParams::resume(path);
        let recorder = RolloutRecorder::new(&rollout_config(config), params).await?;
        Ok(Self {
            recorder,
            history_mode,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        self.recorder.rollout_path()
    }

    /// Queues the items the persistence policy keeps.
    pub(crate) async fn record(&self, items: &[RolloutItem]) {
        let items = persisted_rollout_items(items, self.history_mode);
        if let Err(err) = self.recorder.record_canonical_items(&items).await {
            tracing::warn!("failed to record rollout items: {err}");
        }
    }

    /// Creates the file if needed and writes everything queued so far.
    pub(crate) async fn persist(&self) {
        if let Err(err) = self.recorder.persist().await {
            tracing::warn!("failed to persist rollout: {err}");
        }
    }

    /// Writes everything queued and stops the writer.
    pub(crate) async fn shutdown(&self) {
        if let Err(err) = self.recorder.shutdown().await {
            tracing::warn!("failed to shut down rollout recorder: {err}");
        }
    }
}

fn rollout_config(config: &EngineConfig) -> RolloutConfig {
    RolloutConfig {
        codex_home: config.ether_home.clone(),
        cwd: config.cwd.to_path_buf(),
        model_provider_id: config.model_provider_id.clone(),
        generate_memories: false,
    }
}

pub(crate) fn permission_profile(mode: SandboxMode) -> PermissionProfile {
    match mode {
        SandboxMode::ReadOnly => PermissionProfile::read_only(),
        SandboxMode::WorkspaceWrite => PermissionProfile::workspace_write(),
        SandboxMode::DangerFullAccess => PermissionProfile::Disabled,
    }
}

/// The settings a turn ran with, recorded once per turn as Codex does.
pub(crate) fn turn_context_item(config: &EngineConfig, turn_id: &str) -> TurnContextItem {
    let sandbox_policy = match config.sandbox_mode {
        SandboxMode::ReadOnly => SandboxPolicy::new_read_only_policy(),
        SandboxMode::WorkspaceWrite => SandboxPolicy::new_workspace_write_policy(),
        SandboxMode::DangerFullAccess => SandboxPolicy::DangerFullAccess,
    };
    TurnContextItem {
        turn_id: Some(turn_id.to_string()),
        root_turn_id: Some(turn_id.to_string()),
        disabled_plugin_ids: None,
        cwd: config.cwd.clone(),
        approval_policy: config.approval_policy,
        approvals_reviewer: None,
        sandbox_policy,
        permission_profile: Some(permission_profile(config.sandbox_mode)),
        active_permission_profile: None,
        file_system_sandbox_policy: None,
        model: config.model.slug.clone(),
        comp_hash: None,
        collaboration_mode: None,
        multi_agent_version: None,
        effort: config.reasoning_effort.clone(),
        summary: None,
    }
}

/// Finds the newest session recorded in `config.cwd`.
pub async fn find_latest_session(config: &EngineConfig) -> std::io::Result<Option<PathBuf>> {
    RolloutRecorder::find_latest_thread_path(
        &rollout_config(config),
        LIST_PAGE_SIZE,
        /*cursor*/ None,
        ThreadSortKey::UpdatedAt,
        /*allowed_sources*/ &[],
        /*model_providers*/ None,
        &config.model_provider_id,
        Some(config.cwd.as_path()),
    )
    .await
}

/// A recorded session, for a session picker.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub path: PathBuf,
    /// RFC 3339 time of the last update, if known.
    pub updated_at: Option<String>,
    /// The first user message, or another short description.
    pub preview: Option<String>,
}

/// The newest sessions recorded in `config.cwd`, newest first.
pub async fn list_sessions(
    config: &EngineConfig,
    limit: usize,
) -> std::io::Result<Vec<SessionSummary>> {
    let cwd = [config.cwd.as_path().to_path_buf()];
    let page = RolloutRecorder::list_threads(
        &rollout_config(config),
        limit.max(1),
        /*cursor*/ None,
        ThreadSortKey::UpdatedAt,
        SortDirection::Desc,
        /*allowed_sources*/ &[],
        /*model_providers*/ None,
        Some(&cwd),
        &config.model_provider_id,
        /*search_term*/ None,
    )
    .await?;
    Ok(page
        .items
        .into_iter()
        .map(|item| SessionSummary {
            path: item.path,
            updated_at: item.updated_at,
            preview: item.preview.or(item.first_user_message),
        })
        .collect())
}

/// Resolves a session id or a rollout file path to a rollout file.
pub async fn find_session(
    config: &EngineConfig,
    id_or_path: &str,
) -> std::io::Result<Option<PathBuf>> {
    let path = Path::new(id_or_path);
    if path.is_file() {
        return Ok(Some(path.to_path_buf()));
    }
    codex_rollout::find_thread_path_by_id_str(&config.ether_home, id_or_path).await
}
