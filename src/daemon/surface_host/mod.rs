//! Platform-selected surface host for the daemon overlay window.
//!
//! Two arms share the roster view (`overlay::view`), the pure state machine
//! (`overlay::state`), and the geometry math (`overlay::geometry`), and differ
//! only in how the surface is created and how placement, resize, and hover
//! are applied:
//! - `layershell` (Linux/Wayland): the existing `iced_layershell` shell, kept
//!   byte-identical. It anchors a layer surface to a screen edge with
//!   margins.
//! - `winit` (Windows/macOS): a frameless, transparent, always-on-top
//!   `iced` window moved to the computed on-screen position.
//!
//! The domain logic (command resolution, subscriptions, the singleton probe,
//! and the daemon lifecycle) lives in the parent `daemon` module and is
//! shared by both arms; only the shell-specific emission of the effects and
//! the hover-rect source differ here.

use std::process::ExitCode;

use hyprlay_core::config::Config;

use crate::daemon::adapters::auth::OwnAppAuth;

#[cfg(target_os = "linux")]
mod layershell;

#[cfg(not(target_os = "linux"))]
mod winit;

/// Run the platform-selected overlay shell. Returns the process exit code.
pub(crate) fn run(cfg: Config, auth: Option<OwnAppAuth>) -> ExitCode {
    #[cfg(target_os = "linux")]
    {
        layershell::run(cfg, auth)
    }

    #[cfg(not(target_os = "linux"))]
    {
        winit::run(cfg, auth)
    }
}
