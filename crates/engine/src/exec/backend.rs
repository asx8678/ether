// New in ether: the C4 local process backend on codex-utils-pty, in the role of Codex's exec-server local backend and `unified_exec/process.rs` (codex-rs/core/src/unified_exec/process.rs @ 806d973).
//! Spawns one process (PTY or pipes) after wrapping it with the `codex-sandboxing` transform,
//! pumps its output into capped buffers, and tracks its exit. Callers write stdin, collect
//! output up to a deadline, interrupt and kill through [`LocalProcess`].
//!
//! Dropping a [`LocalProcess`] kills the process (the pty crate's handle does this).

use crate::exec::buffer::HeadTailBuffer;
use codex_protocol::models::PermissionProfile;
use codex_sandboxing::SandboxCommand;
use codex_sandboxing::SandboxManager;
use codex_sandboxing::SandboxTransformRequest;
use codex_sandboxing::SandboxType;
use codex_sandboxing::SpawnRequest;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_path_uri::PathUri;
use codex_utils_pty::ChildFds;
use codex_utils_pty::ProcessHandle;
use codex_utils_pty::ProcessSignal;
use codex_utils_pty::SpawnedProcess;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::sync::broadcast;
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// Set in the child environment when the sandbox blocks network access (Codex `spawn.rs`).
const SANDBOX_NETWORK_DISABLED_ENV_VAR: &str = "CODEX_SANDBOX_NETWORK_DISABLED";
/// Set in the child environment when it runs under Seatbelt (Codex `spawn.rs`).
const SANDBOX_ENV_VAR: &str = "CODEX_SANDBOX";
/// How long collection waits for output to close once the process has exited.
const POST_EXIT_CLOSE_WAIT_CAP: Duration = Duration::from_millis(50);

/// Everything needed to launch one process.
pub(crate) struct LaunchRequest<'a> {
    pub(crate) command: &'a [String],
    pub(crate) cwd: &'a AbsolutePathBuf,
    pub(crate) env: HashMap<String, String>,
    pub(crate) tty: bool,
    pub(crate) sandbox: SandboxType,
    pub(crate) permissions: &'a PermissionProfile,
    pub(crate) sandbox_policy_cwd: &'a AbsolutePathBuf,
    /// `codex-linux-sandbox`, required when `sandbox` is `LinuxSeccomp`.
    pub(crate) linux_sandbox_exe: Option<&'a Path>,
}

/// Exit state of a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExitStatus {
    Running,
    /// Exit code; -1 when the process ended without reporting one.
    Exited(i32),
}

#[derive(Default)]
struct OutputState {
    /// Output not yet returned to the model.
    pending: HeadTailBuffer,
    /// All output, for the end event and sandbox-denial checks.
    aggregated: HeadTailBuffer,
    closed: bool,
}

/// Shared output of a process; outlives the process handle so watchers can read it.
#[derive(Clone)]
pub(crate) struct OutputHandle {
    state: Arc<StdMutex<OutputState>>,
    notify: Arc<Notify>,
    exit_rx: watch::Receiver<ExitStatus>,
}

impl OutputHandle {
    pub(crate) fn exit_status(&self) -> ExitStatus {
        *self.exit_rx.borrow()
    }

    /// All output so far, with an omission marker if the cap dropped bytes.
    pub(crate) fn aggregated_text(&self) -> String {
        let bytes = match self.state.lock() {
            Ok(state) => state.aggregated.to_bytes_with_omission_marker(),
            Err(_) => Vec::new(),
        };
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Waits until the process exits and its output closes (or a short grace period passes).
    pub(crate) async fn wait_for_exit(&self) -> i32 {
        let mut exit_rx = self.exit_rx.clone();
        let code = loop {
            if let ExitStatus::Exited(code) = *exit_rx.borrow_and_update() {
                break code;
            }
            if exit_rx.changed().await.is_err() {
                break -1;
            }
        };
        let deadline = Instant::now() + POST_EXIT_CLOSE_WAIT_CAP;
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.state.lock().map(|state| state.closed).unwrap_or(true) {
                break;
            }
            tokio::select! {
                _ = &mut notified => {}
                _ = tokio::time::sleep_until(deadline) => break,
            }
        }
        code
    }

    /// Drains output that arrives before `deadline`, returning early once the process has exited
    /// and its output is closed. Ported from Codex's `collect_output_until_deadline`, without the
    /// elicitation pause. Returns early (with what was collected) when `cancel` fires.
    pub(crate) async fn collect_until(
        &self,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> HeadTailBuffer {
        let mut collected = HeadTailBuffer::default();
        let mut post_exit_deadline: Option<Instant> = None;
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let (drained, closed) = match self.state.lock() {
                Ok(mut state) => (std::mem::take(&mut state.pending), state.closed),
                Err(_) => (HeadTailBuffer::default(), true),
            };
            let has_output = drained.total_bytes() > 0;
            collected.push_buffer(drained);
            if has_output {
                if Instant::now() >= deadline {
                    break;
                }
                continue;
            }
            let exited = matches!(self.exit_status(), ExitStatus::Exited(_));
            if exited && closed {
                break;
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let wait_until = if exited {
                let post_exit = *post_exit_deadline
                    .get_or_insert_with(|| deadline.min(now + POST_EXIT_CLOSE_WAIT_CAP));
                if now >= post_exit {
                    break;
                }
                post_exit
            } else {
                deadline
            };
            let mut exit_rx = self.exit_rx.clone();
            tokio::select! {
                _ = &mut notified => {}
                _ = exit_rx.changed(), if !exited => {}
                _ = tokio::time::sleep_until(wait_until) => {
                    if !exited {
                        break;
                    }
                }
                _ = cancel.cancelled() => break,
            }
        }
        collected
    }
}

/// A running (or finished) local process.
pub(crate) struct LocalProcess {
    handle: ProcessHandle,
    output: OutputHandle,
    /// Live output chunks for streaming events; taken once.
    deltas: Option<broadcast::Receiver<Vec<u8>>>,
    sandbox: SandboxType,
}

impl LocalProcess {
    pub(crate) fn output(&self) -> &OutputHandle {
        &self.output
    }

    pub(crate) fn sandbox(&self) -> SandboxType {
        self.sandbox
    }

    /// Live output chunks, for streaming events. The channel closes when the output does.
    pub(crate) fn take_deltas(&mut self) -> Option<broadcast::Receiver<Vec<u8>>> {
        self.deltas.take()
    }

    pub(crate) async fn write(&self, data: &[u8]) -> Result<(), String> {
        self.handle
            .writer_sender()
            .send(data.to_vec())
            .await
            .map_err(|_| "stdin is closed".to_string())
    }

    pub(crate) fn interrupt(&self) -> Result<(), String> {
        self.handle
            .signal(ProcessSignal::Interrupt)
            .map_err(|err| err.to_string())
    }

    pub(crate) fn terminate(&self) {
        self.handle.terminate();
    }
}

/// Wraps the command with the sandbox transform and spawns it.
pub(crate) async fn spawn(request: LaunchRequest<'_>) -> Result<LocalProcess, String> {
    let LaunchRequest {
        command,
        cwd,
        env,
        tty,
        sandbox,
        permissions,
        sandbox_policy_cwd,
        linux_sandbox_exe,
    } = request;
    let Some((program, args)) = command.split_first() else {
        return Err("missing command line".to_string());
    };
    let sandbox_policy_cwd = PathUri::from_abs_path(sandbox_policy_cwd);
    let exec_request = SandboxManager::new()
        .transform(SandboxTransformRequest {
            command: SandboxCommand {
                program: OsString::from(program),
                args: args.to_vec(),
                cwd: PathUri::from_abs_path(cwd),
                env,
                additional_permissions: None,
            },
            permissions,
            sandbox,
            environment_id: None,
            sandbox_policy_cwd: &sandbox_policy_cwd,
            sandbox_exe: linux_sandbox_exe,
            use_legacy_landlock: false,
        })
        .map_err(|err| err.to_string())?;
    let mut env = exec_request.env;
    // Lifted from Codex's `ExecRequest::from_sandbox_exec_request`.
    if !exec_request
        .permission_profile
        .network_sandbox_policy()
        .is_enabled()
    {
        env.insert(
            SANDBOX_NETWORK_DISABLED_ENV_VAR.to_string(),
            "1".to_string(),
        );
    }
    if sandbox == SandboxType::MacosSeatbelt {
        env.insert(SANDBOX_ENV_VAR.to_string(), "seatbelt".to_string());
    }
    let spawned = codex_sandboxing::spawn_process(SpawnRequest {
        command: &exec_request.command,
        cwd: cwd.as_path(),
        env: &env,
        arg0: &exec_request.arg0,
        sandbox,
        tty,
        stdin_open: tty,
        inherited_fds: ChildFds::Inherited(&[]),
    })
    .await
    .map_err(|err| format!("failed to spawn process: {err}"))?;
    Ok(start(spawned, sandbox))
}

fn start(spawned: SpawnedProcess, sandbox: SandboxType) -> LocalProcess {
    let SpawnedProcess {
        session: handle,
        mut stdout_rx,
        mut stderr_rx,
        exit_rx,
    } = spawned;
    let state = Arc::new(StdMutex::new(OutputState::default()));
    let notify = Arc::new(Notify::new());
    let (exit_tx, watch_rx) = watch::channel(ExitStatus::Running);
    let (deltas, deltas_rx) = broadcast::channel(256);

    // Output pump: stdout and stderr merged into the buffers, then broadcast for events.
    tokio::spawn({
        let state = Arc::clone(&state);
        let notify = Arc::clone(&notify);
        async move {
            let mut stdout_open = true;
            let mut stderr_open = true;
            while stdout_open || stderr_open {
                let (chunk, from_stdout) = tokio::select! {
                    chunk = stdout_rx.recv(), if stdout_open => (chunk, true),
                    chunk = stderr_rx.recv(), if stderr_open => (chunk, false),
                };
                let Some(chunk) = chunk else {
                    if from_stdout {
                        stdout_open = false;
                    } else {
                        stderr_open = false;
                    }
                    continue;
                };
                if let Ok(mut state) = state.lock() {
                    state.pending.push_chunk(&chunk);
                    state.aggregated.push_chunk(&chunk);
                }
                let _ = deltas.send(chunk);
                notify.notify_waiters();
            }
            if let Ok(mut state) = state.lock() {
                state.closed = true;
            }
            notify.notify_waiters();
        }
    });

    // Exit watcher.
    tokio::spawn({
        let notify = Arc::clone(&notify);
        async move {
            let code = exit_rx.await.unwrap_or(-1);
            let _ = exit_tx.send(ExitStatus::Exited(code));
            notify.notify_waiters();
        }
    });

    LocalProcess {
        handle,
        output: OutputHandle {
            state,
            notify,
            exit_rx: watch_rx,
        },
        deltas: Some(deltas_rx),
        sandbox,
    }
}
