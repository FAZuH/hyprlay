//! Compositor integration: the seam between hyprlay and whatever Wayland
//! compositor is running. Everything the rest of the app needs to know
//! about outputs goes through [`Compositor`], so adding another
//! compositor means adding one adapter — no call-site changes.

mod hyprland;

pub use hyprland::Hyprland;

/// One connected output as reported by the compositor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    pub name: String,
    pub description: String,
    /// Whether this is the currently focused output.
    pub active: bool,
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
        Box::new(Hyprland)
    } else {
        Box::new(Unknown)
    }
}

/// Placeholder for non-Hyprland sessions: reports nothing.
struct Unknown;

impl Compositor for Unknown {
    fn monitors(&self) -> Vec<Monitor> {
        Vec::new()
    }
}
