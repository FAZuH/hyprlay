//! Windows startup adapter: daemon lifecycle + autostart behind the
//! core-owned [`ServiceManager`] port; [`SystemControl`] adapts it to the
//! [`DaemonControl`] the fronts inject.
//!
//! There is no service manager on Windows for "hyprlay is a background
//! process", so the autostart is the simplest mechanism with no service or
//! scheduled task: a `hyprlay.cmd` launcher in the user Startup folder. This
//! matches the ADR's "Run key or Startup folder" latitude; the Run-key was
//! not chosen because it needs registry code and offers no lifecycle benefit
//! over a Startup-folder file.
//!
//! Lifecycle `start`/`stop` have no service manager to drive, so a "start"
//! falls back to spawning the sibling daemon and a "stop" to the ctl-socket
//! quit — the same two primitives the tray uses. Only the autostart install
//! is Windows-specific.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use hyprlay_core::bins::DAEMON_BIN;
use hyprlay_core::daemon_control::Action;
use hyprlay_core::daemon_control::DaemonControl;
use hyprlay_core::daemon_control::ServiceError;
use hyprlay_core::daemon_control::ServiceManager;
use hyprlay_core::domain::Command as DaemonCommand;
use hyprlay_core::platform::Platform;

use super::fs_util;
use crate::platform::ipc::control::Control;

/// The Startup-folder launcher file name.
pub const STARTUP_SCRIPT: &str = "hyprlay.cmd";

/// The Windows startup backend.
pub struct WindowsService;

impl WindowsService {
    fn startup_dir() -> Option<PathBuf> {
        dirs::data_dir().map(|data| data.join("Microsoft/Windows/Start Menu/Programs/Startup"))
    }

    fn script_path() -> Option<PathBuf> {
        Self::startup_dir().map(|dir| dir.join(STARTUP_SCRIPT))
    }

    /// The sibling daemon image, with the `.exe` suffix Windows requires.
    fn daemon_exe(exe_dir: &Path) -> PathBuf {
        exe_dir.join(format!("{DAEMON_BIN}.exe"))
    }

    /// A launcher that detaches the daemon from the login shell so the login
    /// session is not blocked on it, minimizing any flash console window.
    fn launcher(exe_dir: &Path) -> String {
        format!(
            "@start \"\" /min \"{}\"\n",
            Self::daemon_exe(exe_dir).display()
        )
    }
}

impl ServiceManager for WindowsService {
    fn unit_installed(&self) -> bool {
        Self::script_path().is_some_and(|path| path.exists())
    }

    fn systemctl(&self, subcommand: &str) -> Result<(), ServiceError> {
        match subcommand {
            // No service manager: starting "via the service" is a spawn, and
            // stopping it is a socket quit (the only reliable teardown).
            "start" => self.spawn_daemon(),
            "stop" => self.quit_via_socket(),
            other => Err(ServiceError::UnsupportedSubcommand {
                backend: "Windows startup",
                subcommand: other.to_string(),
            }),
        }
    }

    fn spawn_daemon(&self) -> Result<(), ServiceError> {
        let exe = std::env::current_exe().map_err(|source| ServiceError::LocateExe { source })?;
        let Some(dir) = exe.parent() else {
            return Err(ServiceError::NoExeParent { exe: exe.clone() });
        };
        let path = Self::daemon_exe(dir);
        if !path.exists() {
            return Err(ServiceError::DaemonMissing { path });
        }
        let mut cmd = Command::new(&path);
        // CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW + null stdio + no wait:
        // the daemon must outlive this binary and never hold a console.
        crate::platform::host::host()
            .spawn(&mut cmd)
            .map_err(|source| ServiceError::SpawnDaemon { source })
    }

    fn quit_via_socket(&self) -> Result<(), ServiceError> {
        hyprlay_core::ctl::send_command_line(&Control, &DaemonCommand::Quit.to_string())
            .map(|_| ())
            .ok_or(ServiceError::DaemonUnreachable)
    }

    fn install(
        &self,
        exe_dir: &Path,
        _config_base: &Path,
        _data_base: &Path,
        _start: bool,
    ) -> Result<Vec<String>, ServiceError> {
        let path = Self::script_path().ok_or(ServiceError::ResolveDir {
            what: "Startup folder",
        })?;
        fs_util::write_file(&path, Self::launcher(exe_dir).as_bytes())?;
        Ok(vec![format!("wrote {}", path.display())])
    }

    fn uninstall(
        &self,
        _config_base: &Path,
        _data_base: &Path,
    ) -> Result<Vec<String>, ServiceError> {
        let mut report = Vec::new();
        if let Some(path) = Self::script_path() {
            fs_util::remove_reported(&path, &mut report)?;
        }
        Ok(report)
    }
}

/// The [`DaemonControl`] the fronts inject (see the [`systemd`] counterpart).
pub struct SystemControl;

impl DaemonControl for SystemControl {
    fn unit_installed(&self) -> bool {
        WindowsService.unit_installed()
    }

    fn perform(&self, action: Action) -> Result<(), ServiceError> {
        match action {
            Action::SystemctlStart => WindowsService.systemctl("start"),
            Action::SystemctlStop => WindowsService.systemctl("stop"),
            Action::SpawnDaemon => WindowsService.spawn_daemon(),
            Action::SocketQuit => WindowsService.quit_via_socket(),
        }
    }
}
