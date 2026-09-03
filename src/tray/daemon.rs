//! Tray-to-front spawning for the tray front.
//!
//! The daemon Start/Stop *routing* (toggle→action planning and the
//! systemctl / socket / daemon-spawn boundary) now lives in
//! `hyprlay_core::daemon_control` so every front shares one source of truth.
//! What remains here is the tray-specific settings-window launch:
//! `spawn_sibling_gui` runs the tray's own image (`hyprlay`) with the `gui`
//! subcommand.

use hyprlay_core::bins::GUI_APP_ID;
use hyprlay_core::bins::GUI_LOCK;
use hyprlay_core::platform::Platform;
use hyprlay_core::singleton::AcquireError;

/// Open the settings window (`hyprlay gui`) via the tray's own image.
///
/// If the GUI is already running (its `flock` is held) the function does not
/// spawn a second copy. Instead it tries to bring the existing window to the
/// front via `hyprctl dispatch focuswindow` on Hyprland. On other compositors
/// or when `hyprctl` is unavailable it returns an explanatory error so the
/// caller can surface it instead of failing silently.
pub fn spawn_sibling_gui() -> Result<(), String> {
    // Fast-path: GUI already running → focus instead of spawn.
    match hyprlay_core::singleton::acquire(GUI_LOCK) {
        Err(AcquireError::AlreadyHeld) => return focus_existing_gui(),
        Ok(_lock) => {
            // No GUI running. Drop the probe lock immediately — the real GUI
            // will acquire its own lock on startup. The gap between drop and
            // spawn is a benign TOCTOU: at worst two GUIs race and one loses
            // the flock.
            drop(_lock);
        }
        Err(AcquireError::Io(_)) => {
            // Could not probe the lock (e.g. XDG_RUNTIME_DIR missing). Fall
            // through to spawn and let that path report the real error.
        }
    }

    // The `hyprlay` binary always exists next to the running tray: it is the
    // tray's own image. Resolve it once for both the hyprctl and direct paths.
    let exe = std::env::current_exe()
        .map_err(|e| format!("error: could not locate the running hyprlay binary: {e}"))?;

    // If the tray has no WAYLAND_DISPLAY but we are on Hyprland, the child
    // would inherit a headless env and fail to open a window. Route the
    // launch through the compositor so it gets the right environment.
    if std::env::var_os("WAYLAND_DISPLAY").is_none() && is_hyprland() {
        match exec_via_hyprctl(&exe, "gui") {
            Ok(()) => return Ok(()),
            Err(e) => {
                // Fall through to direct spawn; the hyprctl failure is
                // already descriptive but direct spawn may still succeed
                // (e.g. X11 fallback).
                tracing::warn!(
                    event = "tray_hyprctl_exec_failed",
                    error = %e,
                    "hyprctl dispatch exec failed, falling back to direct spawn"
                );
            }
        }
    }

    spawn_gui(&exe)
}

/// Try to focus an already-running GUI window via hyprctl.
///
/// Only meaningful on Hyprland. On other compositors or when hyprctl is not
/// installed the error explains the already-running state.
fn focus_existing_gui() -> Result<(), String> {
    if !is_hyprland() {
        return Err(
            "hyprlay-gui is already running (could not focus: not a Hyprland session)".to_string(),
        );
    }
    // Prefer class match on the stable application ID. Fall back to title if
    // the compositor uses a different class naming.
    let attempts = [
        format!("class:{GUI_APP_ID}"),
        "title:hyprlay".to_string(),
        format!("class:^{GUI_APP_ID}$"),
    ];
    let mut last_err = String::new();
    for ident in &attempts {
        match hyprctl_focus(ident) {
            Ok(()) => return Ok(()),
            Err(e) => last_err = e,
        }
    }
    Err(format!(
        "hyprlay-gui is already running but could not be focused (tried hyprctl dispatch focuswindow): {last_err}"
    ))
}

fn hyprctl_focus(ident: &str) -> Result<(), String> {
    let output = std::process::Command::new("hyprctl")
        .args(["dispatch", "focuswindow", ident])
        .output()
        .map_err(|e| format!("could not run hyprctl: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if detail.is_empty() {
            // hyprctl sometimes reports failure on stdout.
            let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if out.is_empty() {
                format!("exit {}", output.status)
            } else {
                out
            }
        } else {
            detail
        };
        Err(format!(
            "hyprctl dispatch focuswindow {ident} failed: {detail}"
        ))
    }
}

fn is_hyprland() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
            || crate::platform::compositor::hyprland::has_socket()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

fn exec_via_hyprctl(exe: &std::path::Path, arg: &str) -> Result<(), String> {
    let exe_str = exe.display().to_string();
    let output = std::process::Command::new("hyprctl")
        .args(["dispatch", "exec", &exe_str, arg])
        .output()
        .map_err(|e| format!("could not run hyprctl: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if detail.is_empty() {
            let out = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if out.is_empty() {
                format!("exit {}", output.status)
            } else {
                out
            }
        } else {
            detail
        };
        Err(format!(
            "hyprctl dispatch exec {exe_str} {arg} failed: {detail}"
        ))
    }
}

/// Run the settings window (`hyprlay gui`) detached so it outlives (and
/// never holds) the tray's terminal/process group.
fn spawn_gui(exe: &std::path::Path) -> Result<(), String> {
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("gui");
    // Own process group + null stdio + no wait: the child must outlive the
    // tray and never hold its terminal.
    crate::platform::host::host()
        .spawn(&mut cmd)
        .map_err(|e| format!("error: could not start hyprlay gui: {e}"))
}

#[cfg(test)]
mod tests {
    use hyprlay_core::daemon_control::Action;
    use hyprlay_core::daemon_control::DaemonControl;
    use hyprlay_core::daemon_control::StopPolicy;
    use hyprlay_core::daemon_control::Toggle;
    use hyprlay_core::daemon_control::execute_toggle;
    use hyprlay_core::daemon_control::plan_action;

    #[test]
    fn an_installed_unit_routes_start_through_systemctl() {
        assert_eq!(
            plan_action(Toggle::Start, true, StopPolicy::ViaSocket),
            Action::SystemctlStart
        );
    }

    #[test]
    fn a_stop_always_quits_over_the_control_socket() {
        // Stopping goes through the socket regardless of supervisor/unit
        // presence: the daemon could have been started by a supervisor, a
        // sibling, or a socket-activated unit.
        assert_eq!(
            plan_action(Toggle::Stop, true, StopPolicy::ViaSocket),
            Action::SocketQuit
        );
        assert_eq!(
            plan_action(Toggle::Stop, false, StopPolicy::ViaSocket),
            Action::SocketQuit
        );
    }

    #[test]
    fn without_a_unit_start_spawns_the_sibling_daemon() {
        assert_eq!(
            plan_action(Toggle::Start, false, StopPolicy::ViaSocket),
            Action::SpawnDaemon
        );
    }

    /// Spy at the process/socket boundary: records what ran so tests verify
    /// state, not call mechanics.
    #[derive(Default)]
    struct FakeControl {
        installed: bool,
        fail_with: Option<String>,
        performed: std::sync::Mutex<Vec<Action>>,
    }

    impl FakeControl {
        fn performed(&self) -> Vec<Action> {
            self.performed.lock().unwrap().clone()
        }
    }

    impl DaemonControl for FakeControl {
        fn unit_installed(&self) -> bool {
            self.installed
        }

        fn perform(&self, action: Action) -> Result<(), String> {
            self.performed.lock().unwrap().push(action);
            match &self.fail_with {
                Some(text) => Err(text.clone()),
                None => Ok(()),
            }
        }
    }

    #[test]
    fn a_successful_toggle_runs_exactly_one_action_and_stays_quiet() {
        let control = FakeControl {
            installed: true,
            ..FakeControl::default()
        };
        let outcome = execute_toggle(&control, Toggle::Start, StopPolicy::ViaSocket);
        assert_eq!(outcome, None);
        assert_eq!(control.performed(), vec![Action::SystemctlStart]);
    }

    #[test]
    fn a_failed_action_surfaces_its_error_text() {
        let control = FakeControl {
            installed: false,
            fail_with: Some("error: could not start hyprlayd: ENOENT".into()),
            ..FakeControl::default()
        };
        let outcome = execute_toggle(&control, Toggle::Stop, StopPolicy::ViaSocket);
        assert_eq!(
            outcome,
            Some("error: could not start hyprlayd: ENOENT".into())
        );
        assert_eq!(control.performed(), vec![Action::SocketQuit]);
    }
}
