//! Compositor integration: the seam between hyprlay and whatever Wayland
//! compositor is running. Everything the rest of the app needs to know
//! about outputs goes through [`Compositor`], so adding another
//! compositor means adding one adapter — no call-site changes.

mod hyprland;

pub use hyprland::Hyprland;

/// One connected output as reported by the compositor.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Monitor {
    pub name: String,
    pub description: String,
    /// Whether this is the currently focused output.
    pub active: bool,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

pub trait Compositor: Send {
    /// All connected outputs. Empty when none are reported or discovery
    /// fails (failures are traced inside the adapter).
    fn monitors(&self) -> Vec<Monitor>;
}

/// Pick the adapter matching the running session. Hyprland is the only
/// supported compositor today; anything else yields an empty monitor list
/// instead of shelling out to `hyprctl` on foreign compositors.
pub fn detect() -> Box<dyn Compositor> {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        return Box::new(Hyprland);
    }
    if hyprland::has_socket() {
        return Box::new(Hyprland);
    }
    Box::new(Unknown)
}

pub fn cursor_pos() -> Option<(i32, i32)> {
    hyprland::cursor_pos()
}

/// Placeholder for non-Hyprland sessions: reports nothing.
struct Unknown;

impl Compositor for Unknown {
    fn monitors(&self) -> Vec<Monitor> {
        Vec::new()
    }
}
