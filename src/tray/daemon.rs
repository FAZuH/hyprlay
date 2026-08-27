//! Sibling-binary spawning for the tray front.
//!
//! The daemon Start/Stop *routing* (toggle→action planning and the
//! systemctl / socket / daemon-spawn boundary) now lives in
//! `hyprlay_core::daemon_control` so every front shares one source of truth.
//! What remains here is the tray-specific sibling spawn: opening the settings
//! GUI via `spawn_sibling_gui`. Unifying this sibling-binary surface (the
//! `DAEMON_BIN` / `GUI_BIN` consts and the generic `spawn_sibling`) with the
//! other fronts is tracked separately.

use std::process::Stdio;

use hyprlay_core::bins::GUI_BIN;

/// Spawn the sibling `hyprlay-gui` (Open settings).
pub fn spawn_sibling_gui() -> Result<(), String> {
    spawn_sibling(GUI_BIN)
}

/// Resolve `name` next to this binary's image and spawn it detached so it
/// outlives (and never holds) the tray's terminal/process group.
pub fn spawn_sibling(name: &str) -> Result<(), String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("error: could not locate the running hyprlay-tray binary: {e}"))?;
    let Some(dir) = exe.parent() else {
        return Err(format!(
            "error: could not find the directory of {}",
            exe.display()
        ));
    };
    let path = dir.join(name);
    if !path.exists() {
        return Err(format!(
            "error: {name} not found next to hyprlay-tray (expected {})\nthe hyprlay binaries must be installed together",
            path.display()
        ));
    }
    use std::os::unix::process::CommandExt;
    std::process::Command::new(&path)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("error: could not start {name}: {e}"))
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
