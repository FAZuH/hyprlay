//! Hyprland cursor adapter: global cursor position via the Hyprland IPC
//! socket first, then `hyprctl cursorpos` as a fallback. Pure parsing lives
//! in `hyprlay_core::compositor::parse_cursor_reply`, shared with every
//! cursor backend.

use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use hyprlay_core::compositor::CursorSource;
use hyprlay_core::compositor::parse_cursor_reply;

use crate::platform::compositor::hyprland::candidate_socket_paths;

/// Marker type for the Hyprland cursor adapter.
pub struct Hyprland;

impl CursorSource for Hyprland {
    fn cursor_pos(&self) -> Option<(i32, i32)> {
        for path in candidate_socket_paths() {
            if let Some(pos) = cursor_pos_from_socket(&path) {
                return Some(pos);
            }
        }
        cursor_pos_via_hyprctl()
    }
}

fn cursor_pos_from_socket(path: &Path) -> Option<(i32, i32)> {
    let mut stream = match std::os::unix::net::UnixStream::connect(path) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(event = "cursor_socket_failed", path = %path.display(), error = %e, "could not connect to hyprland socket");
            return None;
        }
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
    if let Err(e) = stream.write_all(b"cursorpos\n") {
        tracing::debug!(event = "cursor_send_failed", error = %e, "could not send cursorpos");
        return None;
    }
    let mut buf = Vec::new();
    if let Err(e) = stream.read_to_end(&mut buf) {
        tracing::debug!(event = "cursor_read_failed", error = %e, "could not read cursorpos reply");
        return None;
    }
    let reply = String::from_utf8_lossy(&buf).trim().to_string();
    if reply.is_empty() {
        tracing::debug!(event = "cursor_empty_reply", path = %path.display(), "empty cursorpos reply");
        return None;
    }
    if let Some(pos) = parse_cursor_reply(&reply) {
        return Some(pos);
    }
    tracing::debug!(event = "cursor_parse_failed", reply = %reply, "failed to parse cursorpos");
    None
}

fn cursor_pos_via_hyprctl() -> Option<(i32, i32)> {
    for args in [vec!["cursorpos", "-j"], vec!["cursorpos"]] {
        if let Ok(out) = std::process::Command::new("hyprctl").args(&args).output()
            && out.status.success()
        {
            let reply = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !reply.is_empty() {
                if let Some(pos) = parse_cursor_reply(&reply) {
                    return Some(pos);
                }
                tracing::debug!(event = "cursor_hyprctl_parse_failed", reply = %reply, "hyprctl parse failed");
            }
        }
    }
    None
}
