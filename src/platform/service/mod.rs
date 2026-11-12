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

mod fs_util;

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

/// Whether an autostart registration for this OS is already installed.
#[cfg(target_os = "linux")]
pub fn registration_installed() -> bool {
    Systemd.unit_installed()
}

/// Whether an autostart registration for this OS is already installed.
#[cfg(target_os = "macos")]
pub fn registration_installed() -> bool {
    Launchd.unit_installed()
}

/// Whether an autostart registration for this OS is already installed.
#[cfg(target_os = "windows")]
pub fn registration_installed() -> bool {
    WindowsService.unit_installed()
}

/// Rewrite the autostart registration for freshly replaced binaries and get
/// the daemon running again. `hyprlay update` calls this only when a
/// registration already exists, so an update never installs one.
#[cfg(target_os = "linux")]
pub fn refresh_registration(
    exe_dir: &Path,
    config_base: &Path,
    data_base: &Path,
) -> Result<Vec<String>, ServiceError> {
    Systemd.install(exe_dir, config_base, data_base, true)
}

/// Rewrite the autostart registration for freshly replaced binaries and get
/// the daemon running again.
#[cfg(target_os = "macos")]
pub fn refresh_registration(
    exe_dir: &Path,
    config_base: &Path,
    data_base: &Path,
) -> Result<Vec<String>, ServiceError> {
    // `bootstrap` refuses an already-loaded label, so drop it first. A
    // missing registration is tolerated by uninstall already.
    let _ = Launchd.uninstall(config_base, data_base);
    Launchd.install(exe_dir, config_base, data_base, true)
}

/// Rewrite the autostart registration for freshly replaced binaries and get
/// the daemon running again.
#[cfg(target_os = "windows")]
pub fn refresh_registration(
    exe_dir: &Path,
    config_base: &Path,
    data_base: &Path,
) -> Result<Vec<String>, ServiceError> {
    let mut report = WindowsService.install(exe_dir, config_base, data_base, false)?;
    // There is no service manager to start: the daemon was stopped so its
    // image could be replaced, so bring it back explicitly.
    WindowsService.spawn_daemon()?;
    report.push("started hyprlayd".to_string());
    Ok(report)
}

/// Quit the running daemon over the control socket, however it was started.
#[cfg(target_os = "linux")]
pub fn quit_daemon() -> Result<(), ServiceError> {
    Systemd.quit_via_socket()
}

/// Quit the running daemon over the control socket, however it was started.
#[cfg(target_os = "macos")]
pub fn quit_daemon() -> Result<(), ServiceError> {
    Launchd.quit_via_socket()
}

/// Quit the running daemon over the control socket, however it was started.
#[cfg(target_os = "windows")]
pub fn quit_daemon() -> Result<(), ServiceError> {
    WindowsService.quit_via_socket()
}
