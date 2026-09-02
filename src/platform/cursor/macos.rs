//! macOS cursor adapter: global pointer position via `NSEvent.mouseLocation`.
//!
//! `mouseLocation` returns the pointer in the global display coordinate space
//! with a bottom-left origin, in points (a plain query — no event tap, so no
//! accessibility permission). It is flipped to the top-left origin the overlay
//! rect uses via the pure `normalize_cursor_pos` converter and the main
//! display's logical height (`CGDisplayBounds` of the main display, any
//! thread).

use core_graphics::display::CGDisplayBounds;
use core_graphics::display::CGMainDisplayID;
use hyprlay_core::compositor::CursorSource;
use hyprlay_core::compositor::normalize_cursor_pos;
use objc2_app_kit::NSEvent;

/// Marker type for the macOS cursor adapter.
pub struct Macos;

impl CursorSource for Macos {
    fn cursor_pos(&self) -> Option<(i32, i32)> {
        let p = NSEvent::mouseLocation(); // bottom-left Y-up global points
        let bounds = CGDisplayBounds(CGMainDisplayID());
        let screen_h = bounds.size.height as i32;
        Some(normalize_cursor_pos(
            (p.x as i32, p.y as i32),
            1.0,
            Some(screen_h),
        ))
    }
}
