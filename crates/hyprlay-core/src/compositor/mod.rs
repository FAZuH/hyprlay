//! Compositor + cursor integration: the pure port seam between hyprlay and
//! whatever Wayland compositor (or OS cursor backend) is running. Everything
//! the rest of the app needs to know about outputs and the global cursor goes
//! through these two traits, so adding another compositor or cursor backend
//! means adding one adapter — no call-site changes.
//!
//! This crate is platform-neutral: the concrete adapters live in the host
//! package's `src/platform/` module and are selected there by the
//! `detect()` factories. Only the port traits, the shared wire-parsing
//! helper, and the no-op fallbacks live here.

/// One connected output as reported by the compositor.
#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    pub name: String,
    pub description: String,
    /// Whether this is the currently focused output.
    pub active: bool,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f32,
}

impl Default for Monitor {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            active: false,
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            scale: 1.0,
        }
    }
}

pub trait Compositor: Send {
    /// All connected outputs. Empty when none are reported or discovery
    /// fails (failures are traced inside the adapter).
    fn monitors(&self) -> Vec<Monitor>;
}

/// Global cursor position as reported by the OS / compositor, in the same
/// coordinate space as [`Monitor`].
///
/// Returns `None` where there is no portable global-cursor read — e.g. a
/// non-Hyprland Wayland session, which has no standard global-cursor query.
/// That degrades `dim-on-hover` to a no-op, exactly as the design requires.
pub trait CursorSource: Send + Sync {
    fn cursor_pos(&self) -> Option<(i32, i32)>;
}

/// Placeholder for unsupported compositor sessions: reports no monitors.
pub struct Unknown;

impl Compositor for Unknown {
    fn monitors(&self) -> Vec<Monitor> {
        Vec::new()
    }
}

/// No-op cursor source for compositors with no global-cursor query.
pub struct NoCursor;

impl CursorSource for NoCursor {
    fn cursor_pos(&self) -> Option<(i32, i32)> {
        None
    }
}

/// Parse the text form of a `cursorpos` reply: either `x, y` or JSON
/// `{"x":..,"y":..}`. Pure and shared by every cursor adapter.
pub fn parse_cursor_reply(reply: &str) -> Option<(i32, i32)> {
    let reply = reply.trim();
    if reply.is_empty() {
        return None;
    }
    if reply.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(reply) {
            let x = v.get("x").and_then(|x| x.as_i64()).map(|x| x as i32);
            let y = v.get("y").and_then(|y| y.as_i64()).map(|y| y as i32);
            if let (Some(x), Some(y)) = (x, y) {
                return Some((x, y));
            }
        }
        return None;
    }
    let parts: Vec<&str> = reply.split(',').collect();
    if parts.len() == 2
        && let (Ok(x), Ok(y)) = (
            parts[0].trim().parse::<i32>(),
            parts[1].trim().parse::<i32>(),
        )
    {
        return Some((x, y));
    }
    None
}

/// Convert a physical-pixel length to logical pixels for a given scale factor
/// (Windows and X11 report physical pixels; winit positions in logical
/// points). Clamp a degenerate 0 scale to 1.0.
pub fn physical_to_logical(px: i32, scale: f32) -> i32 {
    let scale = if scale == 0.0 { 1.0 } else { scale };
    (px as f32 / scale).round() as i32
}

/// Flip a top-left-origin Y coordinate to the bottom-left-origin Y-up space
/// that macOS uses for global display coordinates. `screen_h` is the logical
/// height of the enclosing config-space (the primary display height in points).
pub fn macos_flip_y(y: i32, screen_h: i32) -> i32 {
    screen_h - y
}

/// Normalize a raw global cursor position into the overlay's top-left-origin
/// logical screen space.
///
/// `raw` is the adapter's native output. `scale` is the physical→logical
/// factor (1.0 when the query already returns logical points, as on macOS).
/// `flip_h` carries the logical display height used to convert a bottom-left
/// Y-up origin (macOS only); `None` leaves Y as-is (top-left-origin physical
/// pixels, as on Windows/X11).
pub fn normalize_cursor_pos(raw: (i32, i32), scale: f32, flip_h: Option<i32>) -> (i32, i32) {
    let (x, y) = raw;
    let x = physical_to_logical(x, scale);
    let y = match flip_h {
        Some(h) => macos_flip_y(physical_to_logical(y, scale), h),
        None => physical_to_logical(y, scale),
    };
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_compositor_reports_no_monitors() {
        assert!(Unknown.monitors().is_empty());
    }

    #[test]
    fn no_cursor_source_reports_none() {
        assert_eq!(NoCursor.cursor_pos(), None);
    }

    #[test]
    fn cursor_reply_parses_comma_form() {
        assert_eq!(parse_cursor_reply("100, 200"), Some((100, 200)));
        assert_eq!(parse_cursor_reply("  -10 ,  20  "), Some((-10, 20)));
        assert_eq!(parse_cursor_reply("0,0"), Some((0, 0)));
    }

    #[test]
    fn cursor_reply_parses_json_form() {
        assert_eq!(
            parse_cursor_reply(r#"{"x": 123, "y": 456}"#),
            Some((123, 456))
        );
        assert_eq!(parse_cursor_reply(r#"{"x":-5,"y":10}"#), Some((-5, 10)));
    }

    #[test]
    fn cursor_reply_rejects_garbage() {
        assert_eq!(parse_cursor_reply(""), None);
        assert_eq!(parse_cursor_reply("not a point"), None);
        assert_eq!(parse_cursor_reply(r#"{"a":1}"#), None);
    }

    #[test]
    fn physical_to_logical_rounds_to_nearest_logical_pixel() {
        assert_eq!(physical_to_logical(1920, 1.25), 1536);
        assert_eq!(physical_to_logical(1920, 1.0), 1920);
        assert_eq!(physical_to_logical(1920, 0.0), 1920); // degenerate scale
        assert_eq!(physical_to_logical(-100, 2.0), -50);
    }

    #[test]
    fn macos_flip_y_converts_top_left_to_bottom_left_origin() {
        // A point 100px from the top of a 1080 logical space is 980 from the
        // bottom under the macOS Y-up convention.
        assert_eq!(macos_flip_y(100, 1080), 980);
        assert_eq!(macos_flip_y(0, 1080), 1080);
        assert_eq!(macos_flip_y(1080, 1080), 0);
    }

    #[test]
    fn normalize_cursor_top_left_origin_scales_down() {
        // Windows/X11 (top-left origin, physical px): divide by scale.
        assert_eq!(normalize_cursor_pos((1920, 1200), 1.5, None), (1280, 800));
        // Scale 1.0 is a no-op.
        assert_eq!(normalize_cursor_pos((500, 300), 1.0, None), (500, 300));
    }

    #[test]
    fn normalize_cursor_bottom_left_origin_flips_y() {
        // macOS (bottom-left Y-up, points): flip by the display height.
        assert_eq!(
            normalize_cursor_pos((100, 100), 1.0, Some(1080)),
            (100, 980)
        );
        // A scale factor still applies before the flip when physical pixels
        // are involved (documents the per-arm semantics).
        assert_eq!(
            normalize_cursor_pos((100, 200), 2.0, Some(1080)),
            (50, 1080 - 100)
        );
    }
}
