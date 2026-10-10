// New in ether: terminal plumbing for the TUI (frame scheduling and input events from Codex's `tui/`).
mod frame_rate_limiter;
pub(crate) mod frame_requester;

pub(crate) use frame_requester::FrameRequester;
