// New in ether: a minimal Chrome DevTools Protocol client over `--remote-debugging-pipe`.
//! Chrome reads protocol messages on file descriptor 3 and writes on 4, each a JSON object
//! ended by a NUL byte. Using the pipe instead of `--remote-debugging-port` means no listening
//! socket: only this process can drive the browser. When the pipe closes (also when ether
//! exits or crashes), Chrome shuts down with its helper processes.

use serde_json::Value;
use serde_json::json;
use std::collections::VecDeque;
use std::io::PipeReader;
use std::io::PipeWriter;
use std::io::Read;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

/// A headless Chrome driven over the DevTools pipe. Killed, with its profile removed, on drop.
pub(crate) struct Browser {
    child: Child,
    to_chrome: PipeWriter,
    from_chrome: PipeReader,
    /// Bytes read but not yet split into messages, and how much of it was searched for NUL.
    buffer: Vec<u8>,
    scanned: usize,
    next_id: u64,
    /// Events that arrived while waiting for a command's response.
    events: VecDeque<Value>,
    profile: PathBuf,
}

impl Browser {
    /// Starts `chrome` headless with a throwaway profile and the given extra flags.
    pub(crate) fn launch(chrome: &Path, args: &[String]) -> Result<Self, String> {
        let profile = std::env::temp_dir().join(format!(
            "ether-chrome-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        std::fs::create_dir_all(&profile).map_err(|err| format!("Chrome profile: {err}"))?;
        let (chrome_reads, to_chrome) = std::io::pipe().map_err(|err| format!("pipe: {err}"))?;
        let (from_chrome, chrome_writes) = std::io::pipe().map_err(|err| format!("pipe: {err}"))?;
        let read_fd = chrome_reads.as_raw_fd();
        let write_fd = chrome_writes.as_raw_fd();
        let mut command = Command::new(chrome);
        command
            .arg("--headless")
            .arg("--remote-debugging-pipe")
            .arg(format!("--user-data-dir={}", profile.display()))
            .args(args)
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // SAFETY: between fork and exec this only calls `fcntl` and `dup2`, which are
        // async-signal-safe, and allocates nothing.
        unsafe {
            command.pre_exec(move || {
                // Copy both ends above 10 first so neither is clobbered when placed at 3 and 4.
                let read_copy = libc::fcntl(read_fd, libc::F_DUPFD_CLOEXEC, 10);
                let write_copy = libc::fcntl(write_fd, libc::F_DUPFD_CLOEXEC, 10);
                if read_copy < 0
                    || write_copy < 0
                    || libc::dup2(read_copy, 3) < 0
                    || libc::dup2(write_copy, 4) < 0
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().map_err(|err| {
            let _ = std::fs::remove_dir_all(&profile);
            format!("cannot start Chrome: {err}")
        })?;
        drop(chrome_reads);
        drop(chrome_writes);
        Ok(Self {
            child,
            to_chrome,
            from_chrome,
            buffer: Vec::new(),
            scanned: 0,
            next_id: 0,
            events: VecDeque::new(),
            profile,
        })
    }

    /// The throwaway profile directory, also used for the pages ether writes.
    pub(crate) fn profile(&self) -> &Path {
        &self.profile
    }

    /// Sends a command (to a page when `session` is set) and returns its result.
    pub(crate) fn call(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
        timeout: Duration,
    ) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        let mut bytes = serde_json::to_vec(&message).map_err(|err| format!("{method}: {err}"))?;
        bytes.push(0);
        self.to_chrome
            .write_all(&bytes)
            .map_err(|err| format!("{method}: Chrome is gone ({err})"))?;
        let deadline = Instant::now() + timeout;
        loop {
            let message = self.read_message(deadline, method)?;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = message.get("error") {
                    return Err(format!("{method}: {error}"));
                }
                return Ok(message.get("result").cloned().unwrap_or(Value::Null));
            }
            self.events.push_back(message);
        }
    }

    /// Waits for the event `method` from `session`.
    pub(crate) fn wait_event(
        &mut self,
        method: &str,
        session: &str,
        timeout: Duration,
    ) -> Result<Value, String> {
        let matches = |message: &Value| {
            message.get("method").and_then(Value::as_str) == Some(method)
                && message.get("sessionId").and_then(Value::as_str) == Some(session)
        };
        if let Some(index) = self.events.iter().position(matches) {
            return Ok(self.events.remove(index).unwrap_or(Value::Null));
        }
        let deadline = Instant::now() + timeout;
        loop {
            let message = self.read_message(deadline, method)?;
            if matches(&message) {
                return Ok(message);
            }
            self.events.push_back(message);
        }
    }

    /// Drops the events received so far.
    pub(crate) fn clear_events(&mut self) {
        self.events.clear();
    }

    fn read_message(&mut self, deadline: Instant, waiting_for: &str) -> Result<Value, String> {
        loop {
            if let Some(offset) = self.buffer[self.scanned..]
                .iter()
                .position(|byte| *byte == 0)
            {
                let end = self.scanned + offset;
                let message = serde_json::from_slice(&self.buffer[..end])
                    .map_err(|err| format!("bad DevTools message: {err}"));
                self.buffer.drain(..=end);
                self.scanned = 0;
                return message;
            }
            self.scanned = self.buffer.len();
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() || !poll_readable(self.from_chrome.as_raw_fd(), remaining)? {
                return Err(format!("{waiting_for}: Chrome did not answer in time"));
            }
            let mut chunk = vec![0_u8; 1 << 16];
            let count = self
                .from_chrome
                .read(&mut chunk)
                .map_err(|err| format!("{waiting_for}: {err}"))?;
            if count == 0 {
                return Err(format!("{waiting_for}: Chrome closed the DevTools pipe"));
            }
            self.buffer.extend_from_slice(&chunk[..count]);
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

/// Whether `fd` becomes readable within `timeout`.
fn poll_readable(fd: i32, timeout: Duration) -> Result<bool, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let mut pollfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let millis = deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(i32::MAX as u128) as i32;
        // SAFETY: one valid pollfd, owned for the duration of the call.
        let result = unsafe { libc::poll(&mut pollfd, 1, millis) };
        if result >= 0 {
            return Ok(result > 0);
        }
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(format!("poll: {err}"));
        }
    }
}
