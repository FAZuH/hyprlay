//! Client-side installation of the hyprlay user service: writes the systemd
//! **user** unit and the XDG `.desktop` entry next to the running binaries,
//! then drives `systemctl --user`. Like `help`, both commands are answered
//! before any socket connect — they manage the on-disk install, not the
//! running daemon.
//!
//! Seam layout: the writers and flow functions are pure with respect to the
//! filesystem they are handed (base dirs + exe dir come in as parameters),
//! and every systemctl call goes through the injectable [`Systemctl`]
//! runner, so tests pin exact file contents and command sequences from
//! tempdir roots without shelling out.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use hyprlay_core::bins::DAEMON_BIN;
use hyprlay_core::bins::GUI_BIN;
use hyprlay_core::bins::TRAY_BIN;

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
        let output = std::process::Command::new("systemctl")
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
fn tray_unit_text(exe_dir: &Path) -> String {
    format!(
        "[Unit]\n\
         Description=hyprlay system tray menu\n\
         \n\
         [Service]\n\
         ExecStart={}/{}\n\
         Restart=on-failure\n\
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
    config_base: &Path,
    data_base: &Path,
    exe_dir: &Path,
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

/// Stop + disable the unit (a missing unit is fine), then delete both
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
    remove_reported(&unit_path(config_base), &mut report)?;
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

/// CLI adapter for `hyprlay install`: resolves the real XDG roots and exe
/// dir, runs the flow against real systemctl, prints the report. Exit code
/// 0 on success, 1 on failure.
pub fn run_install(no_start: bool) -> i32 {
    let Some(config_base) = dirs::config_dir() else {
        eprintln!(
            "error: could not determine the config directory (XDG_CONFIG_HOME or $HOME/.config)"
        );
        return 1;
    };
    let Some(data_base) = dirs::data_dir() else {
        eprintln!(
            "error: could not determine the data directory (XDG_DATA_HOME or $HOME/.local/share)"
        );
        return 1;
    };
    let Some(exe_dir) = std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
    else {
        eprintln!("error: could not locate the running hyprlay binary");
        return 1;
    };
    match install(
        &config_base,
        &data_base,
        &exe_dir,
        !no_start,
        &RealSystemctl,
    ) {
        Ok(report) => {
            for line in report {
                println!("{line}");
            }
            0
        }
        Err(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}

/// CLI adapter for `hyprlay uninstall`; same contract as [`run_install`].
pub fn run_uninstall() -> i32 {
    let Some(config_base) = dirs::config_dir() else {
        eprintln!(
            "error: could not determine the config directory (XDG_CONFIG_HOME or $HOME/.config)"
        );
        return 1;
    };
    let Some(data_base) = dirs::data_dir() else {
        eprintln!(
            "error: could not determine the data directory (XDG_DATA_HOME or $HOME/.local/share)"
        );
        return 1;
    };
    match uninstall(&config_base, &data_base, &RealSystemctl) {
        Ok(report) => {
            for line in report {
                println!("{line}");
            }
            0
        }
        Err(message) => {
            eprintln!("error: {message}");
            1
        }
    }
}
