//! Linux systemd adapter: the real process/socket boundary behind the daemon
//! Start/Stop toggle, plus the install/uninstall of the user units and the
//! desktop entry. Implements the core-owned [`ServiceManager`] port; the
//! [`SystemControl`] wrapper adapts it to the [`DaemonControl`] the fronts
//! already inject.
//!
//! Behaviour is byte-identical to the old implementation: the same systemctl
//! invocation, the same detached sibling spawn, the same socket quit, the same
//! unit/desktop templates and report lines, the same systemctl command
//! sequence. Only the location moved (host CLI + core → this host adapter) so
//! `hyprlay-core` stays platform-neutral and every OS's install/autostart
//! routes through the same `ServiceManager` port.

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

use hyprlay_core::bins::DAEMON_BIN;
use hyprlay_core::bins::GUI_BIN;
use hyprlay_core::bins::TRAY_BIN;
use hyprlay_core::daemon_control::Action;
use hyprlay_core::daemon_control::DaemonControl;
use hyprlay_core::daemon_control::SERVICE_UNIT;
use hyprlay_core::daemon_control::ServiceManager;
use hyprlay_core::domain::Command as DaemonCommand;
use hyprlay_core::platform::Platform;

use crate::platform::ipc::control::Control;

/// The systemd user-startup backend.
pub struct Systemd;

impl ServiceManager for Systemd {
    fn unit_installed(&self) -> bool {
        // Exit success is the whole verdict; the printed unit is noise.
        Command::new("systemctl")
            .args(["--user", "cat", SERVICE_UNIT])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn systemctl(&self, subcommand: &str) -> Result<(), String> {
        let output = Command::new("systemctl")
            .args(["--user", subcommand, SERVICE_UNIT])
            .output()
            .map_err(|e| format!("error: could not run systemctl: {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if detail.is_empty() {
            format!("exit {}", output.status)
        } else {
            detail
        };
        Err(format!("error: systemctl {subcommand} failed: {detail}"))
    }

    fn spawn_daemon(&self) -> Result<(), String> {
        let exe = std::env::current_exe()
            .map_err(|e| format!("error: could not locate the running hyprlay binary: {e}"))?;
        let Some(dir) = exe.parent() else {
            return Err(format!(
                "error: could not find the directory of {}",
                exe.display()
            ));
        };
        let path = dir.join(DAEMON_BIN);
        if !path.exists() {
            return Err(format!(
                "error: {DAEMON_BIN} not found next to the running hyprlay binary (expected {})\n\
                 the hyprlay binaries must be installed together",
                path.display()
            ));
        }
        let mut cmd = Command::new(&path);
        // Own process group + null stdio + no wait: the daemon must outlive
        // this binary and never hold its terminal. A double start is already
        // guarded daemon-side by the socket probe.
        crate::platform::host::host()
            .spawn(&mut cmd)
            .map_err(|e| format!("error: could not start {DAEMON_BIN}: {e}"))
    }

    fn quit_via_socket(&self) -> Result<(), String> {
        hyprlay_core::ctl::send_command_line(&Control, &DaemonCommand::Quit.to_string())
            .map(|_| ())
            .ok_or_else(|| "error: daemon unreachable".to_string())
    }

    fn install(
        &self,
        exe_dir: &Path,
        config_base: &Path,
        data_base: &Path,
        start: bool,
    ) -> Result<Vec<String>, String> {
        install(exe_dir, config_base, data_base, start, &RealSystemctl)
    }

    fn uninstall(&self, config_base: &Path, data_base: &Path) -> Result<Vec<String>, String> {
        uninstall(config_base, data_base, &RealSystemctl)
    }
}

/// The [`DaemonControl`] the fronts inject. A unit struct (the backend is a
/// ZST) so existing `&SystemControl` / `Arc::new(SystemControl)` call sites
/// keep working.
pub struct SystemControl;

impl DaemonControl for SystemControl {
    fn unit_installed(&self) -> bool {
        Systemd.unit_installed()
    }

    fn perform(&self, action: Action) -> Result<(), String> {
        match action {
            Action::SystemctlStart => Systemd.systemctl("start"),
            Action::SystemctlStop => Systemd.systemctl("stop"),
            Action::SpawnDaemon => Systemd.spawn_daemon(),
            Action::SocketQuit => Systemd.quit_via_socket(),
        }
    }
}

/// The systemctl boundary: `run(args)` executes `systemctl --user <args…>`.
/// Owned wrapper around an external binary, so tests substitute a recording
/// double instead of invoking real units.
pub trait Systemctl {
    fn run(&self, args: &[&str]) -> Result<(), String>;
}

/// Production runner: captures exit status and stderr so flow errors carry
/// systemctl's own explanation.
struct RealSystemctl;

impl Systemctl for RealSystemctl {
    fn run(&self, args: &[&str]) -> Result<(), String> {
        let joined = args.join(" ");
        let output = Command::new("systemctl")
            .arg("--user")
            .args(args)
            .output()
            .map_err(|e| format!("{joined}: could not run systemctl: {e}"))?;
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr.trim();
            if detail.is_empty() {
                Err(format!("{joined}: exited with {}", output.status))
            } else {
                Err(format!("{joined}: {detail}"))
            }
        }
    }
}

fn unit_path(config_base: &Path) -> PathBuf {
    config_base.join("systemd/user/hyprlay.service")
}

fn desktop_path(data_base: &Path) -> PathBuf {
    data_base.join("applications/hyprlay.desktop")
}

fn tray_unit_path(config_base: &Path) -> PathBuf {
    config_base.join("systemd/user/hyprlay-tray.service")
}

fn unit_text(exe_dir: &Path) -> String {
    // %h is systemd's own home specifier, resolved by the manager at load
    // time — credentials may arrive via service.env even under systemd.
    format!(
        "[Unit]\n\
         Description=hyprlay overlay daemon\n\
         \n\
         [Service]\n\
         ExecStart={}/{}\n\
         Restart=on-failure\n\
         EnvironmentFile=-%h/.config/hyprlay/service.env\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exe_dir.display(),
        DAEMON_BIN
    )
}

fn desktop_text(exe_dir: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=hyprlay\n\
         Exec={}/{}\n\
         Terminal=false\n",
        exe_dir.display(),
        GUI_BIN
    )
}

/// Second systemd user unit, mirroring the daemon template
/// (`Restart=on-failure`, `EnvironmentFile`, `WantedBy=default.target`) so a
/// late-starting waybar still gets its tray. Runs the resident tray binary.
/// `PassEnvironment` ensures the tray (and any GUI it spawns) inherits the
/// compositor environment even when started early by systemd.
fn tray_unit_text(exe_dir: &Path) -> String {
    format!(
        "[Unit]\n\
         Description=hyprlay system tray menu\n\
         \n\
         [Service]\n\
         ExecStart={}/{}\n\
         Restart=on-failure\n\
         PassEnvironment=WAYLAND_DISPLAY DISPLAY XDG_RUNTIME_DIR HYPRLAND_INSTANCE_SIGNATURE\n\
         EnvironmentFile=-%h/.config/hyprlay/service.env\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exe_dir.display(),
        TRAY_BIN
    )
}

/// The sibling binaries that must all be present before anything is written.
/// A partial install would otherwise reference missing executables.
const REQUIRED_BINS: &[&str] = &[DAEMON_BIN, GUI_BIN, TRAY_BIN];

/// Names of the required sibling binaries missing from `exe_dir`.
fn missing_bins(exe_dir: &Path) -> Vec<&'static str> {
    REQUIRED_BINS
        .iter()
        .copied()
        .filter(|name| !exe_dir.join(name).exists())
        .collect()
}

/// Write both unit files + the desktop entry, reload the user manager, and
/// (unless `start` is false) enable + start both units. Every step lands in
/// the returned report; the first failing step aborts with an error naming
/// it.
///
/// Binaries are verified **before any write**: a partial install must fail
/// loudly naming the missing bins rather than writing units that reference
/// executables that are not there.
pub fn install(
    exe_dir: &Path,
    config_base: &Path,
    data_base: &Path,
    start: bool,
    systemctl: &dyn Systemctl,
) -> Result<Vec<String>, String> {
    let missing = missing_bins(exe_dir);
    if !missing.is_empty() {
        return Err(format!(
            "error: missing binaries for install: {}\nthe hyprlay binaries must be installed together",
            missing.join(", ")
        ));
    }

    let unit = unit_path(config_base);
    let tray_unit = tray_unit_path(config_base);
    let desktop = desktop_path(data_base);
    write_file(&unit, &unit_text(exe_dir))?;
    write_file(&tray_unit, &tray_unit_text(exe_dir))?;
    write_file(&desktop, &desktop_text(exe_dir))?;

    let mut report = vec![
        format!("wrote {}", unit.display()),
        format!("wrote {}", tray_unit.display()),
        format!("wrote {}", desktop.display()),
    ];

    systemctl
        .run(&["daemon-reload"])
        .map_err(|e| format!("systemctl --user daemon-reload failed: {e}"))?;
    report.push("systemctl --user daemon-reload: ok".to_string());

    if start {
        systemctl
            .run(&["enable", "--now", "hyprlay"])
            .map_err(|e| format!("systemctl --user enable --now hyprlay failed: {e}"))?;
        report.push("systemctl --user enable --now hyprlay: ok".to_string());
        systemctl
            .run(&["enable", "--now", "hyprlay-tray"])
            .map_err(|e| format!("systemctl --user enable --now hyprlay-tray failed: {e}"))?;
        report.push("systemctl --user enable --now hyprlay-tray: ok".to_string());
    } else {
        report.push("skipped systemctl --user enable --now (--no-start)".to_string());
    }
    Ok(report)
}

/// Stop + disable the units (a missing unit is fine), then delete the
/// files. Uninstalling twice succeeds identically.
pub fn uninstall(
    config_base: &Path,
    data_base: &Path,
    systemctl: &dyn Systemctl,
) -> Result<Vec<String>, String> {
    let mut report = Vec::new();
    match systemctl.run(&["disable", "--now", "hyprlay"]) {
        Ok(()) => report.push("systemctl --user disable --now hyprlay: ok".to_string()),
        Err(e) => report.push(format!(
            "systemctl --user disable --now hyprlay: tolerated ({e})"
        )),
    }
    match systemctl.run(&["disable", "--now", "hyprlay-tray"]) {
        Ok(()) => report.push("systemctl --user disable --now hyprlay-tray: ok".to_string()),
        Err(e) => report.push(format!(
            "systemctl --user disable --now hyprlay-tray: tolerated ({e})"
        )),
    }
    remove_reported(&unit_path(config_base), &mut report)?;
    remove_reported(&tray_unit_path(config_base), &mut report)?;
    remove_reported(&desktop_path(data_base), &mut report)?;
    Ok(report)
}

fn write_file(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
    }
    fs::write(path, contents).map_err(|e| format!("could not write {}: {e}", path.display()))
}

fn remove_reported(path: &Path, report: &mut Vec<String>) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => report.push(format!("removed {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            report.push(format!("already absent {}", path.display()))
        }
        Err(e) => return Err(format!("could not remove {}: {e}", path.display())),
    }
    Ok(())
}
