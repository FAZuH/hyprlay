//! Windows cursor adapter: global pointer position via `GetCursorPos`.
//!
//! Returns physical pixels, top-left origin, in the global virtual-screen
//! space. The winit arm compares this against the overlay's logical rect; the
//! two coincide at a 100% scale factor. On a hi-DPI display the physical→
//! logical conversion is left to a follow-up that can resolve the window's
//! scale factor (winit does not expose it to the app here), so hover can be
//! off by the DPI scale on scaled displays.

use hyprlay_core::compositor::CursorSource;
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

/// Marker type for the Windows cursor adapter.
pub struct Win32;

impl CursorSource for Win32 {
    fn cursor_pos(&self) -> Option<(i32, i32)> {
        let mut p = POINT { x: 0, y: 0 };
        // Nonzero return = success (BOOL). Physical pixels, top-left origin.
        if unsafe { GetCursorPos(&mut p) } != 0 {
            Some((p.x, p.y))
        } else {
            None
        }
    }
}
