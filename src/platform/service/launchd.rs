//! macOS launchd adapter: daemon lifecycle + autostart behind the core-owned
//! [`ServiceManager`] port; [`SystemControl`] adapts it to the
//! [`DaemonControl`] the fronts inject.
//!
//! The autostart is a LaunchAgent plist in `~/Library/LaunchAgents/` under
//! the label `hyprlay.user` (matching the systemd unit-name convention —
//! `SERVICE_UNIT` is `hyprlay`, so this agent is the same unit under the
//! launchd naming). The plist points launchd at the sibling daemon binary and
//! keeps it alive (`KeepAlive`) so a crash/laptop wake relaunches it. It is
//! registered in the `gui/<uid>` domain with `launchctl bootstrap`.

use std::fs;
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

use crate::platform::ipc::control::Control;

/// The launchd label for the user LaunchAgent (matches the systemd unit name
/// convention).
pub const LAUNCH_AGENT_LABEL: &str = "hyprlay.user";

/// The launchd LaunchAgent backend.
pub struct Launchd;

impl Launchd {
    fn agents_dir() -> Option<PathBuf> {
        dirs::home_dir().map(|home| home.join("Library/LaunchAgents"))
    }

    fn plist_path() -> Option<PathBuf> {
        Self::agents_dir().map(|dir| dir.join(format!("{LAUNCH_AGENT_LABEL}.plist")))
    }

    fn uid() -> u32 {
        // Real user id on macOS. Safety: `getuid` takes no arguments and
        // returns a plain value; libc declares it `unsafe` because FFI.
        unsafe { libc::getuid() }
    }

    /// The plist: run the sibling daemon at login and keep it alive.
    fn plist(exe_dir: &Path) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
             <plist version=\"1.0\">\n\
             <dict>\n\
             \t<key>Label</key>\n\
             \t<string>{LAUNCH_AGENT_LABEL}</string>\n\
             \t<key>ProgramArguments</key>\n\
             \t<array>\n\
             \t\t<string>{}/{}</string>\n\
             \t</array>\n\
             \t<key>RunAtLoad</key>\n\
             \t<true/>\n\
             \t<key>KeepAlive</key>\n\
             \t<true/>\n\
             </dict>\n\
             </plist>\n",
            exe_dir.display(),
            DAEMON_BIN
        )
    }

    fn run_launchctl(&self, args: &[&str]) -> Result<(), ServiceError> {
        let joined = args.join(" ");
        let output = Command::new("launchctl")
            .args(args)
            .output()
            .map_err(|source| ServiceError::CommandNotRun {
                command: joined.clone(),
                program: "launchctl",
                source,
            })?;
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr.trim();
            let detail = if detail.is_empty() {
                format!("exited with {}", output.status)
            } else {
                detail.to_string()
            };
            Err(ServiceError::CommandFailed {
                command: joined,
                detail,
            })
        }
    }
}

impl ServiceManager for Launchd {
    fn unit_installed(&self) -> bool {
        Self::plist_path().is_some_and(|path| path.exists())
    }

    fn systemctl(&self, subcommand: &str) -> Result<(), ServiceError> {
        let uid = Self::uid();
        match subcommand {
            "start" => {
                let plist = Self::plist_path().ok_or(ServiceError::ResolveDir {
                    what: "LaunchAgents dir",
                })?;
                let plist = plist.to_str().ok_or(ServiceError::NonUtf8PlistPath)?;
                // bootstrap loads + starts the agent; "already bootstrapped"
                // means it is running, which is the desired outcome.
                let domain = format!("gui/{uid}");
                let args = vec!["bootstrap", domain.as_str(), plist];
                match self.run_launchctl(&args) {
                    Ok(()) => {}
                    Err(e) if e.to_string().contains("already bootstrapped") => return Ok(()),
                    Err(source) => {
                        return Err(ServiceError::LaunchctlStepFailed {
                            step: "bootstrap",
                            source: Box::new(source),
                        });
                    }
                }
                let label = format!("gui/{uid}/{LAUNCH_AGENT_LABEL}");
                self.run_launchctl(&["enable", label.as_str()])
                    .map_err(|source| ServiceError::LaunchctlStepFailed {
                        step: "enable",
                        source: Box::new(source),
                    })
            }
            "stop" => {
                let label = format!("gui/{uid}/{LAUNCH_AGENT_LABEL}");
                self.run_launchctl(&["bootout", label.as_str()])
            }
            other => Err(ServiceError::UnsupportedSubcommand {
                backend: "launchd",
                subcommand: other.to_string(),
            }),
        }
    }

    fn spawn_daemon(&self) -> Result<(), ServiceError> {
        let exe = std::env::current_exe().map_err(|source| ServiceError::LocateExe { source })?;
        let Some(dir) = exe.parent() else {
            return Err(ServiceError::NoExeParent { exe: exe.clone() });
        };
        let path = dir.join(DAEMON_BIN);
        if !path.exists() {
            return Err(ServiceError::DaemonMissing { path });
        }
        let mut cmd = Command::new(&path);
        // process_group(0) + null stdio + no wait: the daemon must outlive
        // this binary and never hold its terminal.
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
        let path = Self::plist_path().ok_or(ServiceError::ResolveDir {
            what: "LaunchAgents dir",
        })?;
        write_plist(&path, &Self::plist(exe_dir))?;
        let uid = Self::uid();
        let mut report = vec![format!("wrote {}", path.display())];
        let plist = path.to_str().ok_or(ServiceError::NonUtf8PlistPath)?;
        let domain = format!("gui/{uid}");
        let args = vec!["bootstrap", domain.as_str(), plist];
        if let Err(source) = self.run_launchctl(&args) {
            // A fresh install is never already bootstrapped, so a bootstrap
            // failure here is real.
            return Err(ServiceError::LaunchctlInstallStepFailed {
                step: "bootstrap",
                source: Box::new(source),
            });
        }
        report.push(format!("launchctl bootstrap gui/{uid} {plist}: ok"));
        let label = format!("gui/{uid}/{LAUNCH_AGENT_LABEL}");
        self.run_launchctl(&["enable", label.as_str()])
            .map_err(|source| ServiceError::LaunchctlInstallStepFailed {
                step: "enable",
                source: Box::new(source),
            })?;
        report.push(format!(
            "launchctl enable gui/{uid}/{LAUNCH_AGENT_LABEL}: ok"
        ));
        Ok(report)
    }

    fn uninstall(
        &self,
        _config_base: &Path,
        _data_base: &Path,
    ) -> Result<Vec<String>, ServiceError> {
        let uid = Self::uid();
        let mut report = Vec::new();
        let label = format!("gui/{uid}/{LAUNCH_AGENT_LABEL}");
        match self.run_launchctl(&["bootout", label.as_str()]) {
            Ok(()) => report.push(format!(
                "launchctl bootout gui/{uid}/{LAUNCH_AGENT_LABEL}: ok"
            )),
            Err(e) => report.push(format!(
                "launchctl bootout gui/{uid}/{LAUNCH_AGENT_LABEL}: tolerated ({e})"
            )),
        }
        if let Some(path) = Self::plist_path() {
            remove_reported(&path, &mut report)?;
        }
        Ok(report)
    }
}

/// The [`DaemonControl`] the fronts inject (see the [`systemd`] counterpart).
pub struct SystemControl;

impl DaemonControl for SystemControl {
    fn unit_installed(&self) -> bool {
        Launchd.unit_installed()
    }

    fn perform(&self, action: Action) -> Result<(), ServiceError> {
        match action {
            Action::SystemctlStart => Launchd.systemctl("start"),
            Action::SystemctlStop => Launchd.systemctl("stop"),
            Action::SpawnDaemon => Launchd.spawn_daemon(),
            Action::SocketQuit => Launchd.quit_via_socket(),
        }
    }
}

fn write_plist(path: &Path, contents: &str) -> Result<(), ServiceError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ServiceError::CreateDirFailed {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(path, contents).map_err(|source| ServiceError::WriteFileFailed {
        path: path.to_path_buf(),
        source,
    })
}

fn remove_reported(path: &Path, report: &mut Vec<String>) -> Result<(), ServiceError> {
    match fs::remove_file(path) {
        Ok(()) => report.push(format!("removed {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            report.push(format!("already absent {}", path.display()))
        }
        Err(source) => {
            return Err(ServiceError::RemoveFileFailed {
                path: path.to_path_buf(),
                source,
            });
        }
    }
    Ok(())
}
