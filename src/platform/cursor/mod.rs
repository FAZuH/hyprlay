//! Cursor source adapter selection: pick the backend that can report the
//! global pointer position.
//!
//! - Linux/Hyprland reads it over its IPC socket (the fast path).
//! - Linux/X11 reads it with `XQueryPointer`; a Wayland session (no X server)
//!   fails the connect and degrades `dim-on-hover` to a no-op, exactly as the
//!   design requires.
//! - Windows/macOS read it with `GetCursorPos` / `NSEvent.mouseLocation`.
//!
//! Every adapter reports a top-left-origin position in the overlay's logical
//! screen space (macOS flips its bottom-left Y-up point space via the pure
//! `hyprlay_core::compositor` converters). Unsupported targets use the core
//! [`NoCursor`] no-op.

use std::sync::OnceLock;

use hyprlay_core::compositor::CursorSource;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
use hyprlay_core::compositor::NoCursor;

#[cfg(target_os = "linux")]
pub mod hyprland;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod win32;
#[cfg(target_os = "linux")]
pub mod x11;

pub fn detect() -> Box<dyn CursorSource> {
    #[cfg(target_os = "linux")]
    {
        detect_linux()
    }
    #[cfg(target_os = "windows")]
    {
        Box::new(win32::Win32)
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::Macos)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        Box::new(NoCursor)
    }
}

/// Linux cursor selection: Hyprland's socket fast path, then X11. A
/// non-Hyprland Wayland session has no portable global-cursor query (and an
/// Xwayland X server would report a foreign coordinate space), so it degrades
/// to the [`NoCursor`] no-op, exactly as the design requires.
#[cfg(target_os = "linux")]
fn detect_linux() -> Box<dyn CursorSource> {
    use crate::platform::compositor::hyprland::has_socket;

    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() || has_socket() {
        return Box::new(hyprland::Hyprland);
    }
    if std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland")
    {
        return Box::new(NoCursor);
    }
    Box::new(x11::X11)
}

/// The process-wide cursor source, resolved once. `detect()` re-scans the
/// environment / socket dirs, which is far too expensive to redo on every
/// 50 ms hover tick; resolving it once keeps the adapter's per-poll behaviour
/// (e.g. Hyprland's short-lived socket connection) while skipping the
/// per-tick re-selection.
static CURSOR_SOURCE: OnceLock<Box<dyn CursorSource>> = OnceLock::new();

/// Read the global cursor position, or `None` where the platform has no
/// portable global-cursor query. Resolves the [`CursorSource`] once (the
/// process-wide [`CURSOR_SOURCE`]) and polls that instance on each call.
pub fn cursor_pos() -> Option<(i32, i32)> {
    CURSOR_SOURCE.get_or_init(detect).cursor_pos()
}
