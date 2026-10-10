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

use std::sync::RwLock;

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
///
/// A `RwLock` holding an `Option`, not a `OnceLock`: a `OnceLock` cannot be
/// replaced once set, which made the whole `dim-on-hover` feature unreachable
/// from any unit test — a test that set it would lock every other test in the
/// process out. Tests inject through [`set_for_tests`]; production resolves
/// lazily through [`detect`].
static CURSOR_SOURCE: RwLock<Option<Box<dyn CursorSource>>> = RwLock::new(None);

/// Read the global cursor position, or `None` where the platform has no
/// portable global-cursor query. Resolves the [`CursorSource`] once (the
/// process-wide [`CURSOR_SOURCE`]) and polls that instance on each call.
pub fn cursor_pos() -> Option<(i32, i32)> {
    let source = CURSOR_SOURCE.read().unwrap();
    if let Some(source) = source.as_ref() {
        return source.cursor_pos();
    }
    drop(source);
    let mut slot = CURSOR_SOURCE.write().unwrap();
    slot.get_or_insert_with(detect).cursor_pos()
}

/// Inject a cursor source for a unit test. Production never calls this; the
/// daemon resolves through [`detect`].
#[cfg(test)]
pub fn set_for_tests(source: Box<dyn CursorSource>) {
    *CURSOR_SOURCE.write().unwrap() = Some(source);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test that was impossible before: `CURSOR_SOURCE` was a `OnceLock`,
    /// which cannot be replaced once set, so a test that set it would lock
    /// every other test in the process out and the whole `dim-on-hover`
    /// feature was unreachable from any unit test.
    ///
    /// This injects a source through `set_for_tests` and reads it back, and
    /// the `CursorSource` trait is the injection point. A source that returns
    /// a fixed position proves the injection works; the hover *transition*
    /// itself (`Overlay::hover_polling` plus the `clear_hover_if_set` guard)
    /// is the surface arms' code and is tested there.
    #[test]
    fn an_injected_source_is_polled() {
        struct Fixed(Option<(i32, i32)>);
        impl CursorSource for Fixed {
            fn cursor_pos(&self) -> Option<(i32, i32)> {
                self.0
            }
        }

        set_for_tests(Box::new(Fixed(Some((120, 240)))));
        assert_eq!(cursor_pos(), Some((120, 240)));

        set_for_tests(Box::new(Fixed(None)));
        assert_eq!(cursor_pos(), None);
    }

    /// Production resolves lazily through `detect` when nothing is injected,
    /// and the result is stable across calls (the source is resolved once).
    #[test]
    fn production_resolves_lazily_and_stably() {
        // No injection: the first call resolves through detect(). On a box
        // with no compositor that is the NoCursor no-op, and either way the
        // second call must agree with the first.
        let first = cursor_pos();
        assert_eq!(cursor_pos(), first);
    }
}
