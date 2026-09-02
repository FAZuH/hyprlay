//! X11 cursor adapter: global pointer position via `XQueryPointer` on the
//! root window.
//!
//! Returns the pointer in root-window pixels, top-left origin. On a Wayland
//! session there is no X server, so `x11rb::connect` fails and `cursor_pos`
//! returns `None` — `dim-on-hover` degrades to a no-op there (the design
//! requires this for unsupported Wayland). Like the Windows adapter, the
//! physical→logical conversion needs the window's scale factor, which is left
//! to a follow-up; the two coincide at a 100% scale.

use hyprlay_core::compositor::CursorSource;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;

/// Marker type for the X11 cursor adapter.
pub struct X11;

impl CursorSource for X11 {
    fn cursor_pos(&self) -> Option<(i32, i32)> {
        let (conn, screen_num) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots[screen_num].root;
        let reply = conn.query_pointer(root).ok()?.reply().ok()?;
        Some((reply.root_x as i32, reply.root_y as i32))
    }
}
