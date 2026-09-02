//! Client-side installation of the hyprlay user service: resolves the real
//! config/data/exe dirs, runs the platform's install/uninstall flow, prints
//! the report. The platform-specific writing + registration (systemd user
//! units, launchd LaunchAgent, Windows Run-key) lives in the `ServiceManager`
//! adapters under `src/platform/service/`; this front is a thin argument
//! resolver + reporter, so `hyprlay install` behaves identically no matter
//! which OS backend answers.
//!
//! Like `help`, both commands are answered before any socket connect — they
//! manage the on-disk install, not the running daemon.

use std::path::Path;

/// CLI adapter for `hyprlay install`: resolves the real XDG roots and exe
/// dir, runs the platform's install flow, prints the report. Exit code 0 on
/// success, 1 on failure.
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
    match crate::platform::service::install_service(&exe_dir, &config_base, &data_base, !no_start) {
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
    match crate::platform::service::uninstall_service(&config_base, &data_base) {
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
