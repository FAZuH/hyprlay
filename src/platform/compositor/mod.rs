//! Compositor adapter selection: pick the adapter matching the running
//! session. Hyprland is the only supported compositor today; anything else
//! yields an empty monitor list instead of shelling out to `hyprctl` on
//! foreign compositors.

use hyprlay_core::compositor::Compositor;
use hyprlay_core::compositor::Unknown;

#[cfg(target_os = "linux")]
pub mod hyprland;

#[cfg(target_os = "linux")]
pub use hyprland::has_socket as hyprland_has_socket;

pub fn detect() -> Box<dyn Compositor> {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            return Box::new(hyprland::Hyprland);
        }
        if hyprland::has_socket() {
            return Box::new(hyprland::Hyprland);
        }
    }
    Box::new(Unknown)
}
