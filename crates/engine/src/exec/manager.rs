// Modified from OpenAI Codex (codex-rs/core/src/unified_exec/process_manager.rs, tools/orchestrator.rs, tools/runtimes/unified_exec.rs and unified_exec/async_watcher.rs @ 806d973): one local path; the orchestrator's approval → sandbox → attempt → retry sequence inlined for exec; no exec-server, guardian, network approval, stdin approval, hooks, telemetry, shell snapshots or plugin attribution.
//! The per-session process store and the `exec_command` / `write_stdin` flows.

use crate::exec::ExecOutput;
use crate::exec::MAX_BACKGROUND_TERMINAL_TIMEOUT_MS;
use crate::exec::MAX_PROCESSES;
use crate::exec::MAX_YIELD_TIME_MS;
use crate::exec::MIN_EMPTY_YIELD_TIME_MS;
use crate::exec::MIN_YIELD_TIME_MS;
use crate::exec::approval::ApprovalKey;
use crate::exec::approval::ApprovalOutcome;
use crate::exec::approval::ApprovalStore;
use crate::exec::approval::ExecApprovalRequirement;
use crate::exec::approval::canonicalize_command_for_approval;
use crate::exec::approval::request_command_approval;
use crate::exec::approval::sandbox_override_for_first_attempt;
use crate::exec::approval::should_bypass_approval;
use crate::exec::approval::unsandboxed_execution_allowed;
use crate::exec::approval::wants_no_sandbox_approval;
use crate::exec::backend;
use crate::exec::backend::ExitStatus;
use crate::exec::backend::LaunchRequest;
use crate::exec::backend::LocalProcess;
use crate::exec::backend::OutputHandle;
use crate::exec::clamp_yield_time;
use crate::exec::generate_chunk_id;
use crate::exec::policy;
use crate::exec::shell::Shell;
use crate::exec::shell::default_user_shell;
use crate::exec::shell::get_shell_by_model_provided_path;
use crate::session::Session;
use codex_execpolicy::Policy;
use codex_protocol::config_types::SandboxMode;
use codex_protocol::config_types::ShellEnvironmentPolicy;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_protocol::exec_output::StreamOutput;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExecCommandBeginEvent;
use codex_protocol::protocol::ExecCommandEndEvent;
use codex_protocol::protocol::ExecCommandOutputDeltaEvent;
use codex_protocol::protocol::ExecCommandSource;
use codex_protocol::protocol::ExecCommandStatus;
use codex_protocol::protocol::ExecOutputStream;
use codex_protocol::sandbox::SandboxOverride;
use codex_protocol::shell_environment::create_env;
use codex_protocol::shell_environment::set_tool_call_id_env_var;
use codex_sandboxing::SandboxManager;
use codex_sandboxing::SandboxType;
use codex_sandboxing::SandboxablePreference;
use codex_sandboxing::is_likely_sandbox_denied;
use codex_shell_command::parse_command::parse_command;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_output_truncation::approx_tokens_from_byte_count;
use codex_utils_path_uri::PathUri;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Weak;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::sync::broadcast;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::warn;

const UNIFIED_EXEC_ENV: [(&str, &str); 10] = [
    ("NO_COLOR", "1"),
    ("TERM", "dumb"),
    ("LANG", "C.UTF-8"),
    ("LC_CTYPE", "C.UTF-8"),
    ("LC_ALL", "C.UTF-8"),
    ("COLORTERM", ""),
    ("PAGER", "cat"),
    ("GIT_PAGER", "cat"),
    ("GH_PAGER", "cat"),
    ("CODEX_CI", "1"),
];
const INTERRUPT: &str = "\u{3}";
/// How long a fresh process gets to fail fast, so sandbox denials can be retried before any
/// events go out (Codex `EARLY_EXIT_GRACE_PERIOD`).
const EARLY_EXIT_GRACE_PERIOD: Duration = Duration::from_millis(150);
const LINUX_SANDBOX_EXE_NAME: &str = "codex-linux-sandbox";
const RETRY_REASON: &str = "command failed; retry without sandbox?";
/// Cap on one output-delta event, as Codex's `UNIFIED_EXEC_OUTPUT_DELTA_MAX_BYTES`.
const OUTPUT_DELTA_MAX_BYTES: usize = 8192;
/// How long the end event waits for pending output deltas.
const DELTA_DRAIN_WAIT: Duration = Duration::from_millis(100);

/// An `exec_command` call, after argument parsing.
pub(crate) struct ExecCommandParams {
    pub(crate) cmd: String,
    pub(crate) shell: Option<String>,
    pub(crate) workdir: Option<String>,
    pub(crate) tty: bool,
    pub(crate) yield_time_ms: u64,
    pub(crate) max_output_tokens: Option<usize>,
    pub(crate) sandbox_permissions: SandboxPermissions,
    pub(crate) justification: Option<String>,
}

/// A `write_stdin` call.
pub(crate) struct WriteStdinParams {
    pub(crate) session_id: i32,
    pub(crate) chars: String,
    pub(crate) yield_time_ms: u64,
    pub(crate) max_output_tokens: Option<usize>,
}

/// Who is asking, for events and approvals.
pub(crate) struct CallContext<'a> {
    pub(crate) sess: &'a Arc<Session>,
    pub(crate) turn_id: &'a str,
    pub(crate) call_id: &'a str,
    pub(crate) cancel: &'a CancellationToken,
}

struct ProcessEntry {
    process: LocalProcess,
    tty: bool,
    /// Reads and writes against one process must not overlap.
    interaction: Mutex<()>,
}

#[derive(Default)]
struct ProcessStore {
    processes: HashMap<i32, Arc<ProcessEntry>>,
    next_id: i32,
}

/// Unified exec state for one session: the user's shell, exec-policy rules, approvals given
/// for the session, and the processes still running.
pub(crate) struct ExecManager {
    shell: Shell,
    policy: Policy,
    linux_sandbox_exe: Option<PathBuf>,
    store: Mutex<ProcessStore>,
    approvals: Mutex<ApprovalStore>,
}

impl ExecManager {
    pub(crate) fn new(ether_home: &std::path::Path) -> Self {
        let policy = policy::load_exec_policy(ether_home).unwrap_or_else(|err| {
            warn!("ignoring exec-policy rules: {err}");
            Policy::empty()
        });
        Self {
            shell: default_user_shell(),
            policy,
            linux_sandbox_exe: find_linux_sandbox_exe(),
            store: Mutex::new(ProcessStore {
                processes: HashMap::new(),
                next_id: 1000,
            }),
            approvals: Mutex::new(ApprovalStore::default()),
        }
    }

    /// Runs `exec_command`: approval, sandboxed spawn (with an unsandboxed retry when the
    /// policy allows it), then output up to the yield time.
    pub(crate) async fn exec_command(
        &self,
        ctx: &CallContext<'_>,
        params: ExecCommandParams,
    ) -> Result<ExecOutput, String> {
        let config = &ctx.sess.config;
        let approval_policy = config.approval_policy;
        let cwd = match params.workdir.as_deref().filter(|dir| !dir.is_empty()) {
            Some(dir) => config.cwd.join(dir),
            None => config.cwd.clone(),
        };
        let shell = params
            .shell
            .as_ref()
            .map(|shell| get_shell_by_model_provided_path(&PathBuf::from(shell)))
            .unwrap_or_else(|| self.shell.clone());
        let command = shell.derive_exec_args(&params.cmd, /*use_login_shell*/ true);
        let sandbox_permissions = params.sandbox_permissions;

        if sandbox_permissions.requests_sandbox_override()
            && policy::prompt_is_rejected_by_policy(approval_policy).is_some()
        {
            return Err(format!(
                "approval policy is {approval_policy:?}; reject command — you cannot ask for escalated permissions if the approval policy is {approval_policy:?}"
            ));
        }

        // 1) Approval.
        let permissions = permission_profile(config.sandbox_mode)
            .materialize_project_roots_with_workspace_roots(std::slice::from_ref(&config.cwd));
        let file_system_sandbox_policy = permissions.file_system_sandbox_policy();
        let requirement = policy::exec_approval_requirement(
            &self.policy,
            &command,
            approval_policy,
            &permissions,
            sandbox_permissions,
        );
        let key = ApprovalKey {
            command: canonicalize_command_for_approval(&command),
            cwd: &cwd,
            tty: params.tty,
            sandbox_permissions,
        };
        let mut already_approved = false;
        match &requirement {
            ExecApprovalRequirement::Skip { .. } => {}
            ExecApprovalRequirement::Forbidden { reason } => return Err(reason.clone()),
            ExecApprovalRequirement::NeedsApproval { reason } => {
                let reason = reason.clone().or_else(|| params.justification.clone());
                self.approve(ctx, &command, &key, reason).await?;
                already_approved = true;
            }
        }

        // 2) First attempt under the selected sandbox.
        let unsandboxed_allowed = unsandboxed_execution_allowed(&file_system_sandbox_policy);
        let sandbox_override = sandbox_override_for_first_attempt(
            sandbox_permissions,
            &requirement,
            &file_system_sandbox_policy,
            already_approved,
        );
        let sandbox_manager = SandboxManager::new();
        let sandbox = match sandbox_override {
            SandboxOverride::BypassSandboxFirstAttempt => SandboxType::None,
            SandboxOverride::NoOverride | SandboxOverride::EscalatedSandboxWithRestrictions => {
                sandbox_manager.select_initial(
                    &permissions,
                    SandboxablePreference::Auto,
                    /*has_managed_network_requirements*/ false,
                )
            }
        };
        let env = self.child_env(ctx);
        let launch = |sandbox| LaunchRequest {
            command: &command,
            cwd: &cwd,
            env: env.clone(),
            tty: params.tty,
            sandbox,
            permissions: &permissions,
            sandbox_policy_cwd: &config.cwd,
            linux_sandbox_exe: self.linux_sandbox_exe.as_deref(),
        };
        let start = Instant::now();
        let mut process = self.spawn(launch(sandbox)).await?;

        // 3) A fast sandbox denial may be retried without the sandbox.
        if process.sandbox() != SandboxType::None
            && let Ok(exit_code) =
                tokio::time::timeout(EARLY_EXIT_GRACE_PERIOD, process.output().wait_for_exit())
                    .await
            && sandbox_denied(
                process.sandbox(),
                exit_code,
                &process.output().aggregated_text(),
            )
        {
            if !wants_no_sandbox_approval(approval_policy) || !unsandboxed_allowed {
                // Surface the denial with the original output; under OnRequest the model may
                // rerun with `sandbox_permissions: "require_escalated"`.
                return Ok(self
                    .finish_without_store(ctx, &command, &cwd, process, start, &params)
                    .await);
            }
            if !should_bypass_approval(approval_policy, already_approved)
                && !self.approvals.lock().await.is_approved(&key)
            {
                match request_command_approval(
                    ctx.sess,
                    ctx.turn_id,
                    ctx.call_id,
                    &command,
                    &cwd,
                    Some(RETRY_REASON.to_string()),
                    ctx.cancel,
                )
                .await
                {
                    ApprovalOutcome::Approved => {}
                    ApprovalOutcome::ApprovedForSession => {
                        self.approvals.lock().await.approve(&key)
                    }
                    ApprovalOutcome::Rejected(message) => return Err(message),
                }
            }
            process = self.spawn(launch(SandboxType::None)).await?;
        }

        // 4) Run: announce, keep the process alive across interrupts, collect until yield.
        self.run_process(ctx, &command, &cwd, process, start, &params)
            .await
    }

    async fn approve(
        &self,
        ctx: &CallContext<'_>,
        command: &[String],
        key: &ApprovalKey<'_>,
        reason: Option<String>,
    ) -> Result<(), String> {
        if self.approvals.lock().await.is_approved(key) {
            return Ok(());
        }
        let cwd = key.cwd;
        match request_command_approval(
            ctx.sess,
            ctx.turn_id,
            ctx.call_id,
            command,
            cwd,
            reason,
            ctx.cancel,
        )
        .await
        {
            ApprovalOutcome::Approved => Ok(()),
            ApprovalOutcome::ApprovedForSession => {
                self.approvals.lock().await.approve(key);
                Ok(())
            }
            ApprovalOutcome::Rejected(message) => Err(message),
        }
    }

    async fn spawn(&self, request: LaunchRequest<'_>) -> Result<LocalProcess, String> {
        if request.sandbox == SandboxType::LinuxSeccomp && request.linux_sandbox_exe.is_none() {
            return Err(format!(
                "the Linux sandbox helper `{LINUX_SANDBOX_EXE_NAME}` was not found next to the ether executable; install it or run with `--sandbox danger-full-access`"
            ));
        }
        backend::spawn(request)
            .await
            .map_err(|err| format!("exec_command failed: {err}"))
    }

    fn child_env(&self, ctx: &CallContext<'_>) -> HashMap<String, String> {
        let thread_id = ctx.sess.thread_id.to_string();
        let mut env = create_env(&ShellEnvironmentPolicy::default(), Some(&thread_id));
        set_tool_call_id_env_var(&mut env, Some(ctx.call_id));
        for (key, value) in UNIFIED_EXEC_ENV {
            env.insert(key.to_string(), value.to_string());
        }
        env
    }

    async fn run_process(
        &self,
        ctx: &CallContext<'_>,
        command: &[String],
        cwd: &AbsolutePathBuf,
        process: LocalProcess,
        start: Instant,
        params: &ExecCommandParams,
    ) -> Result<ExecOutput, String> {
        let process_id = self.allocate_process_id().await;
        let mut process = process;
        let output = process.output().clone();
        let end_emitted = Arc::new(AtomicBool::new(false));
        let announcement = Announcement {
            sess: Arc::downgrade(ctx.sess),
            turn_id: ctx.turn_id.to_string(),
            call_id: ctx.call_id.to_string(),
            process_id: Some(process_id.to_string()),
            command: command.to_vec(),
            cwd: PathUri::from_abs_path(cwd),
            deltas_done: CancellationToken::new(),
        };
        announcement.begin().await;
        if let Some(deltas) = process.take_deltas() {
            spawn_delta_stream(&announcement, deltas);
        } else {
            announcement.deltas_done.cancel();
        }

        // Persist live processes before waiting, so interrupting the turn cannot drop the last
        // handle and kill a background process.
        let entry = Arc::new(ProcessEntry {
            process,
            tty: params.tty,
            interaction: Mutex::new(()),
        });
        let stored = output.exit_status() == ExitStatus::Running;
        if stored {
            self.store_process(process_id, Arc::clone(&entry)).await;
            // Background end event, for processes that outlive this call.
            let announcement = announcement.clone();
            let output = output.clone();
            let end_emitted = Arc::clone(&end_emitted);
            tokio::spawn(async move {
                let exit_code = output.wait_for_exit().await;
                if !end_emitted.swap(true, Ordering::SeqCst) {
                    announcement.end(&output, exit_code, start.elapsed()).await;
                }
            });
        }

        let deadline = start + Duration::from_millis(clamp_yield_time(params.yield_time_ms));
        let collected = output.collect_until(deadline, ctx.cancel).await;
        if ctx.cancel.is_cancelled() {
            return Err("aborted".to_string());
        }
        let wall_time = start.elapsed();
        let (response_process_id, exit_code) = match output.exit_status() {
            ExitStatus::Running => (Some(process_id), None),
            ExitStatus::Exited(exit_code) => {
                if stored {
                    self.store.lock().await.processes.remove(&process_id);
                }
                // Let the remaining output land before reporting the end.
                let _ = exit_code;
                let exit_code = output.wait_for_exit().await;
                if !end_emitted.swap(true, Ordering::SeqCst) {
                    announcement.end(&output, exit_code, wall_time).await;
                }
                (None, Some(exit_code))
            }
        };
        drop(entry);
        Ok(exec_output(
            &collected,
            wall_time,
            response_process_id,
            exit_code,
            params.max_output_tokens,
        ))
    }

    /// Reports a process that exited during the early-exit check (a sandbox denial that will
    /// not be retried) without storing it.
    async fn finish_without_store(
        &self,
        ctx: &CallContext<'_>,
        command: &[String],
        cwd: &AbsolutePathBuf,
        process: LocalProcess,
        start: Instant,
        params: &ExecCommandParams,
    ) -> ExecOutput {
        let output = process.output().clone();
        let exit_code = output.wait_for_exit().await;
        let collected = output.collect_until(Instant::now(), ctx.cancel).await;
        let wall_time = start.elapsed();
        let announcement = Announcement {
            sess: Arc::downgrade(ctx.sess),
            turn_id: ctx.turn_id.to_string(),
            call_id: ctx.call_id.to_string(),
            process_id: None,
            command: command.to_vec(),
            cwd: PathUri::from_abs_path(cwd),
            deltas_done: CancellationToken::new(),
        };
        announcement.deltas_done.cancel();
        announcement.begin().await;
        announcement.end(&output, exit_code, wall_time).await;
        exec_output(
            &collected,
            wall_time,
            None,
            Some(exit_code),
            params.max_output_tokens,
        )
    }

    /// Runs `write_stdin`: writes to (or polls) a running process and returns new output.
    #[expect(
        clippy::await_holding_invalid_type,
        reason = "reads and writes against one process must not overlap"
    )]
    pub(crate) async fn write_stdin(
        &self,
        ctx: &CallContext<'_>,
        params: WriteStdinParams,
    ) -> Result<ExecOutput, String> {
        let process_id = params.session_id;
        let entry = self
            .store
            .lock()
            .await
            .processes
            .get(&process_id)
            .cloned()
            .ok_or_else(|| format!("Unknown process id {process_id}"))?;
        let _interaction = entry.interaction.lock().await;

        if !params.chars.is_empty() {
            if !entry.tty {
                if params.chars == INTERRUPT {
                    entry.process.interrupt()?;
                } else {
                    return Err("stdin is closed for this session; rerun exec_command with tty=true to keep stdin open".to_string());
                }
            } else {
                entry
                    .process
                    .write(params.chars.as_bytes())
                    .await
                    .map_err(|_| "failed to write to stdin".to_string())?;
                // Give the process a brief window to react before polling.
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        let yield_time_ms = {
            // Empty polls wait longer; non-empty writes stay responsive.
            let time_ms = params.yield_time_ms.max(MIN_YIELD_TIME_MS);
            if params.chars.is_empty() {
                time_ms.clamp(MIN_EMPTY_YIELD_TIME_MS, MAX_BACKGROUND_TERMINAL_TIMEOUT_MS)
            } else {
                time_ms.min(MAX_YIELD_TIME_MS)
            }
        };
        let start = Instant::now();
        let output = entry.process.output().clone();
        let collected = output
            .collect_until(start + Duration::from_millis(yield_time_ms), ctx.cancel)
            .await;
        if ctx.cancel.is_cancelled() {
            return Err("aborted".to_string());
        }
        let wall_time = start.elapsed();
        let (response_process_id, exit_code) = match output.exit_status() {
            ExitStatus::Running => (Some(process_id), None),
            ExitStatus::Exited(exit_code) => {
                self.store.lock().await.processes.remove(&process_id);
                (None, Some(exit_code))
            }
        };
        Ok(exec_output(
            &collected,
            wall_time,
            response_process_id,
            exit_code,
            params.max_output_tokens,
        ))
    }

    async fn allocate_process_id(&self) -> i32 {
        let mut store = self.store.lock().await;
        loop {
            store.next_id = store.next_id.wrapping_add(1).max(1000);
            if !store.processes.contains_key(&store.next_id) {
                return store.next_id;
            }
        }
    }

    /// Stores a live process, pruning the oldest one (exited first) past the cap.
    async fn store_process(&self, process_id: i32, entry: Arc<ProcessEntry>) {
        let pruned = {
            let mut store = self.store.lock().await;
            let pruned = if store.processes.len() >= MAX_PROCESSES {
                let exited = store
                    .processes
                    .iter()
                    .filter(|(_, entry)| {
                        entry.process.output().exit_status() != ExitStatus::Running
                    })
                    .map(|(id, _)| *id)
                    .min();
                let victim = exited.or_else(|| store.processes.keys().min().copied());
                victim.and_then(|id| store.processes.remove(&id))
            } else {
                None
            };
            store.processes.insert(process_id, entry);
            pruned
        };
        if let Some(pruned) = pruned {
            pruned.process.terminate();
        }
    }
}

fn permission_profile(sandbox_mode: SandboxMode) -> PermissionProfile {
    match sandbox_mode {
        SandboxMode::ReadOnly => PermissionProfile::read_only(),
        SandboxMode::WorkspaceWrite => PermissionProfile::workspace_write(),
        SandboxMode::DangerFullAccess => PermissionProfile::Disabled,
    }
}

/// `codex-linux-sandbox` next to the running executable, as Codex's arg0 setup finds it.
fn find_linux_sandbox_exe() -> Option<PathBuf> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.join(LINUX_SANDBOX_EXE_NAME);
    candidate.is_file().then_some(candidate)
}

fn sandbox_denied(sandbox: SandboxType, exit_code: i32, text: &str) -> bool {
    let exec_output = ExecToolCallOutput {
        exit_code,
        stderr: StreamOutput::new(text.to_string()),
        aggregated_output: StreamOutput::new(text.to_string()),
        ..Default::default()
    };
    is_likely_sandbox_denied(sandbox, &exec_output)
}

fn exec_output(
    collected: &crate::exec::buffer::HeadTailBuffer,
    wall_time: Duration,
    process_id: Option<i32>,
    exit_code: Option<i32>,
    max_output_tokens: Option<usize>,
) -> ExecOutput {
    let original_token_count =
        usize::try_from(approx_tokens_from_byte_count(collected.total_bytes()))
            .unwrap_or(usize::MAX);
    ExecOutput {
        chunk_id: generate_chunk_id(),
        wall_time,
        raw_output: collected.to_bytes_with_omission_marker(),
        output_omitted_bytes: collected.omitted_bytes(),
        process_id,
        exit_code,
        original_token_count,
        max_output_tokens,
    }
}

/// Emits the begin, delta and end events for one command.
#[derive(Clone)]
struct Announcement {
    sess: Weak<Session>,
    turn_id: String,
    call_id: String,
    process_id: Option<String>,
    command: Vec<String>,
    cwd: PathUri,
    /// Cancelled once every output delta has been sent, so the end event comes after them.
    deltas_done: CancellationToken,
}

impl Announcement {
    async fn begin(&self) {
        let Some(sess) = self.sess.upgrade() else {
            return;
        };
        let event = ExecCommandBeginEvent {
            call_id: self.call_id.clone(),
            plugin_id: None,
            script_path: None,
            process_id: self.process_id.clone(),
            turn_id: self.turn_id.clone(),
            started_at_ms: chrono::Utc::now().timestamp_millis(),
            command: self.command.clone(),
            cwd: self.cwd.clone(),
            parsed_cmd: parse_command(&self.command),
            source: ExecCommandSource::UnifiedExecStartup,
            interaction_input: None,
        };
        sess.send_event(&self.turn_id, EventMsg::ExecCommandBegin(event))
            .await;
    }

    async fn end(&self, output: &OutputHandle, exit_code: i32, duration: Duration) {
        let _ = tokio::time::timeout(DELTA_DRAIN_WAIT, self.deltas_done.cancelled()).await;
        let Some(sess) = self.sess.upgrade() else {
            return;
        };
        let aggregated_output = output.aggregated_text();
        let status = if exit_code == 0 {
            ExecCommandStatus::Completed
        } else {
            ExecCommandStatus::Failed
        };
        let event = ExecCommandEndEvent {
            call_id: self.call_id.clone(),
            plugin_id: None,
            script_path: None,
            process_id: self.process_id.clone(),
            turn_id: self.turn_id.clone(),
            completed_at_ms: chrono::Utc::now().timestamp_millis(),
            command: self.command.clone(),
            cwd: self.cwd.clone(),
            parsed_cmd: parse_command(&self.command),
            source: ExecCommandSource::UnifiedExecStartup,
            interaction_input: None,
            aggregated_output,
            exit_code,
            duration,
            status,
        };
        sess.send_event(&self.turn_id, EventMsg::ExecCommandEnd(event))
            .await;
    }
}

/// Forwards live output as `ExecCommandOutputDelta` events until the output closes.
fn spawn_delta_stream(announcement: &Announcement, mut rx: broadcast::Receiver<Vec<u8>>) {
    let sess = announcement.sess.clone();
    let turn_id = announcement.turn_id.clone();
    let call_id = announcement.call_id.clone();
    let done = announcement.deltas_done.clone();
    tokio::spawn(async move {
        let _done = done.drop_guard();
        loop {
            let chunk = match rx.recv().await {
                Ok(chunk) => chunk,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            };
            let Some(sess) = sess.upgrade() else {
                break;
            };
            for piece in chunk.chunks(OUTPUT_DELTA_MAX_BYTES) {
                let event = ExecCommandOutputDeltaEvent {
                    call_id: call_id.clone(),
                    stream: ExecOutputStream::Stdout,
                    chunk: piece.to_vec(),
                };
                sess.send_event(&turn_id, EventMsg::ExecCommandOutputDelta(event))
                    .await;
            }
        }
    });
}
