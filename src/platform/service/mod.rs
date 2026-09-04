//! Service-management adapters: how a front starts/stops the daemon and its
//! user service, and how the CLI installs/uninstalls the autostart config.
//! Each OS backend implements the core-owned `ServiceManager` port and exposes
//! a [`SystemControl`] [`DaemonControl`] for the fronts. The concrete backend
//! is selected by `#[cfg]`-gating the module re-export, so a front references
//! `SystemControl` (the platform's own) without ever naming the OS.

#[cfg(target_os = "macos")]
pub mod launchd;
#[cfg(target_os = "linux")]
pub mod systemd;
#[cfg(target_os = "windows")]
pub mod windows;

use std::path::Path;

use hyprlay_core::daemon_control::ServiceError;
use hyprlay_core::daemon_control::ServiceManager;
#[cfg(target_os = "macos")]
pub use launchd::Launchd;
#[cfg(target_os = "macos")]
pub use launchd::SystemControl;
#[cfg(target_os = "linux")]
pub use systemd::SystemControl;
#[cfg(target_os = "linux")]
pub use systemd::Systemd;
#[cfg(target_os = "windows")]
pub use windows::SystemControl;
#[cfg(target_os = "windows")]
pub use windows::WindowsService;

/// Install the autostart service config for the running OS.
#[cfg(target_os = "linux")]
pub fn install_service(
    exe_dir: &Path,
    config_base: &Path,
    data_base: &Path,
    start: bool,
) -> Result<Vec<String>, ServiceError> {
    Systemd.install(exe_dir, config_base, data_base, start)
}

/// Uninstall the autostart service config for the running OS.
#[cfg(target_os = "linux")]
pub fn uninstall_service(
    config_base: &Path,
    data_base: &Path,
) -> Result<Vec<String>, ServiceError> {
    Systemd.uninstall(config_base, data_base)
}

/// Install the autostart service config for the running OS.
#[cfg(target_os = "macos")]
pub fn install_service(
    exe_dir: &Path,
    config_base: &Path,
    data_base: &Path,
    start: bool,
) -> Result<Vec<String>, ServiceError> {
    Launchd.install(exe_dir, config_base, data_base, start)
}

/// Uninstall the autostart service config for the running OS.
#[cfg(target_os = "macos")]
pub fn uninstall_service(
    config_base: &Path,
    data_base: &Path,
) -> Result<Vec<String>, ServiceError> {
    Launchd.uninstall(config_base, data_base)
}

/// Install the autostart service config for the running OS.
#[cfg(target_os = "windows")]
pub fn install_service(
    exe_dir: &Path,
    config_base: &Path,
    data_base: &Path,
    start: bool,
) -> Result<Vec<String>, ServiceError> {
    WindowsService.install(exe_dir, config_base, data_base, start)
}

/// Uninstall the autostart service config for the running OS.
#[cfg(target_os = "windows")]
pub fn uninstall_service(
    config_base: &Path,
    data_base: &Path,
) -> Result<Vec<String>, ServiceError> {
    WindowsService.uninstall(config_base, data_base)
}
