// Modified from prime-agent (crates/pa-tui/src/clipboard.rs and osc52.rs, MIT, Copyright (c) 2025-2026 Prime Intellect Ltd.; see THIRD_PARTY_NOTICES.md): the helper chain, the tmux client probe and the OSC 52 cap, without Windows and with the OSC 52 bytes handed back to the caller.
//! `/copy`: puts text on the clipboard through the first channel that works: the platform
//! helper (pbcopy, termux-clipboard-set, wl-copy, xclip, xsel), tmux's buffer when tmux can
//! forward it, or an OSC 52 request the terminal may honour. Blocking; run it off the UI thread.

use base64::Engine as _;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// The longest encoded OSC 52 payload sent; larger ones can desynchronise the terminal.
const MAX_OSC52_ENCODED: usize = 100_000;

/// How long a helper may take before it is killed.
const HELPER_TIMEOUT: Duration = Duration::from_secs(5);

const POLL: Duration = Duration::from_millis(20);

/// Where the text went.
pub(crate) enum Copied {
    /// A local clipboard helper took it.
    Local,
    /// Over SSH and too large for the terminal channel: it is on the remote machine's clipboard.
    Remote,
    /// tmux took it into its buffer and forwards it to the attached terminal.
    Tmux,
    /// Write these bytes to the terminal; whether it honours them cannot be checked.
    Osc52(Vec<u8>),
}

pub(crate) fn copy(text: &str) -> Result<Copied, String> {
    let remote = ["SSH_CONNECTION", "SSH_CLIENT", "MOSH_CONNECTION"]
        .iter()
        .any(|key| std::env::var_os(key).is_some());
    let fits_osc52 = text.len().div_ceil(3) * 4 <= MAX_OSC52_ENCODED;
    // Over SSH a helper would fill the remote clipboard; the terminal channel reaches the
    // user's own machine, so it goes first when the text fits.
    if !(remote && fits_osc52) && copy_with_helper(text) {
        return Ok(if remote {
            Copied::Remote
        } else {
            Copied::Local
        });
    }
    if !fits_osc52 {
        return Err(
            "too large for the terminal clipboard channel and no clipboard helper worked"
                .to_string(),
        );
    }
    if std::env::var_os("TMUX").is_some() {
        if copy_via_tmux(text) {
            return Ok(Copied::Tmux);
        }
        if tmux_blocks_osc52() {
            return Err("tmux cannot forward this copy to a clipboard; check its set-clipboard option and the attached client's Ms capability".to_string());
        }
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    Ok(Copied::Osc52(
        format!("\x1b]52;c;{encoded}\x07").into_bytes(),
    ))
}

fn copy_with_helper(text: &str) -> bool {
    if cfg!(target_os = "macos") {
        return pipe_to("pbcopy", &[], text);
    }
    if std::env::var_os("TERMUX_VERSION").is_some() && pipe_to("termux-clipboard-set", &[], text) {
        return true;
    }
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    if wayland && pipe_to("wl-copy", &[], text) {
        return true;
    }
    std::env::var_os("DISPLAY").is_some()
        && (pipe_to("xclip", &["-selection", "clipboard"], text)
            || pipe_to("xsel", &["--clipboard", "--input"], text))
}

/// tmux's `set-clipboard` is off or external, so it swallows OSC 52 from programs inside it.
fn tmux_blocks_osc52() -> bool {
    tmux_output(&["show", "-s", "set-clipboard"]).is_some_and(|output| {
        matches!(
            output.trim(),
            "set-clipboard external" | "set-clipboard off"
        )
    })
}

/// Loads the text into tmux's buffer with `-w` (forwarded to the terminal as OSC 52), but only
/// when the client showing this pane says it can set the clipboard (its `Ms` capability).
fn copy_via_tmux(text: &str) -> bool {
    let Ok(pane) = std::env::var("TMUX_PANE") else {
        return false;
    };
    if tmux_output(&["show-options", "-s", "-gv", "set-clipboard"])
        .is_none_or(|value| value.trim() == "off")
    {
        return false;
    }
    let Some(window) = tmux_output(&["display-message", "-p", "-t", &pane, "#{window_id}"]) else {
        return false;
    };
    let Some(clients) = tmux_output(&[
        "list-clients",
        "-F",
        "#{client_activity} #{window_id} #{client_name}",
    ]) else {
        return false;
    };
    // The most recently active client looking at this pane's window.
    let Some(client) = clients
        .lines()
        .filter_map(|line| {
            let (activity, rest) = line.split_once(' ')?;
            let (client_window, client) = rest.split_once(' ')?;
            (client_window == window.trim()).then_some((activity.parse::<u64>().ok()?, client))
        })
        .max_by_key(|(activity, _)| *activity)
        .map(|(_, client)| client.to_string())
    else {
        return false;
    };
    let Some(terminal) = tmux_output(&["show-messages", "-T", "-t", &client]) else {
        return false;
    };
    let can_set_clipboard = terminal.lines().any(|line| {
        line.split_once("Ms: (string) ")
            .is_some_and(|(_, capability)| !capability.trim().is_empty())
    });
    can_set_clipboard && pipe_to("tmux", &["load-buffer", "-w", "-t", &client, "-"], text)
}

fn tmux_output(args: &[&str]) -> Option<String> {
    let mut child = Command::new("tmux")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    // Read on a thread so a full pipe cannot stall the child past the timeout.
    let stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut output = String::new();
        stdout
            .and_then(|mut pipe| pipe.read_to_string(&mut output).ok())
            .map(|_| output)
    });
    let succeeded = wait_with_timeout(&mut child);
    let output = reader.join().ok().flatten()?;
    succeeded.then_some(output)
}

/// Runs a helper with `text` on its stdin; true when it took all of it and exited cleanly.
fn pipe_to(program: &str, args: &[&str], text: &str) -> bool {
    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let stdin = child.stdin.take();
    let payload = text.as_bytes().to_vec();
    let writer = std::thread::spawn(move || {
        // Dropping stdin afterwards closes it, which is the helper's end of input.
        stdin.is_some_and(|mut stdin| stdin.write_all(&payload).is_ok())
    });
    let succeeded = wait_with_timeout(&mut child);
    // A killed helper closes its stdin, so the writer thread always finishes.
    let wrote = writer.join().unwrap_or(false);
    succeeded && wrote
}

fn wait_with_timeout(child: &mut Child) -> bool {
    let deadline = Instant::now() + HELPER_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}
