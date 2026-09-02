//! Daemon Start/Stop routing shared by every front (GUI, tray, CLI).
//!
//! Fronts may only depend on `hyprlay-core`, never on each other, so the
//! toggle→action planning and the real process/socket boundary live here as
//! one source of truth. Each front keeps its own `DaemonState` / poll state
//! machine; what is shared is the *decision*: given a [`Toggle`] and whether
//! the systemd user unit is installed, which [`Action`] should run, and how
//! each action is performed through a [`DaemonControl`] (backed by a
//! [`ServiceManager`]).
//!
//! The Stop policy differs per front: the GUI stops via `systemctl stop`
//! when the unit is installed, while the tray always quits over the ctl
//! socket (`quit`) so it can tear down a daemon started by any supervisor,
//! sibling, or socket-activated unit. [`StopPolicy`] carries that choice.
//!
//! This module is platform-neutral: the concrete systemctl / spawn / socket
//! primitives live in the host package's `src/platform/service/` adapters.
//! Only the decision logic and the port traits are here.

use std::path::Path;

/// Which direction a toggle press drives the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Start,
    Stop,
}

/// The concrete mechanism a toggle resolves to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    SystemctlStart,
    SystemctlStop,
    SpawnDaemon,
    SocketQuit,
}

/// How a Stop toggle should be carried out. The GUI uses [`ViaSystemctl`];
/// the tray uses [`ViaSocket`] so it never depends on how the daemon was
/// launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopPolicy {
    ViaSystemctl,
    ViaSocket,
}

/// The process/socket boundary of a toggle, abstracted so the decision logic
/// stays testable without touching systemctl or the daemon.
pub trait DaemonControl: Send + Sync {
    fn unit_installed(&self) -> bool;
    fn perform(&self, action: Action) -> Result<(), String>;
}

/// The service-management boundary behind a [`DaemonControl`]: the systemctl
/// invocations, the detached sibling daemon spawn, the socket quit, and the
/// install/uninstall of the autostart config. The host package implements it
/// per OS (systemd on Linux, launchd on macOS, Windows startup on Windows).
///
/// [`install`](Self::install) and [`uninstall`](Self::uninstall) default to a
/// clear "unsupported" error so an OS that cannot host the hyprlay service
/// reports the gap instead of failing opaquely; each real backend overrides
/// them. The install payload (unit / plist / Run-key) is backend-specific, so
/// the caller only supplies the paths and whether to start right away and
/// receives the human-readable report lines back.
pub trait ServiceManager: Send + Sync {
    /// Whether the user service unit for the daemon is installed.
    fn unit_installed(&self) -> bool;
    /// Run `systemctl --user <subcommand> <SERVICE_UNIT>`.
    fn systemctl(&self, subcommand: &str) -> Result<(), String>;
    /// Detached spawn of the sibling daemon binary.
    fn spawn_daemon(&self) -> Result<(), String>;
    /// Quit the running daemon over the control socket.
    fn quit_via_socket(&self) -> Result<(), String>;
    /// Install the autostart service config next to `exe_dir` and register
    /// it; `start` controls whether it begins running immediately. Returns
    /// one human-readable line per step.
    fn install(
        &self,
        exe_dir: &Path,
        config_base: &Path,
        data_base: &Path,
        start: bool,
    ) -> Result<Vec<String>, String> {
        let _ = (exe_dir, config_base, data_base, start);
        Err("install is not supported on this platform".to_string())
    }
    /// Uninstall the autostart service config and deregister it. Returns one
    /// human-readable line per step.
    fn uninstall(&self, config_base: &Path, data_base: &Path) -> Result<Vec<String>, String> {
        let _ = (config_base, data_base);
        Err("uninstall is not supported on this platform".to_string())
    }
}

/// systemd user unit name. Its presence routes Start/Stop through systemctl;
/// its absence falls back to spawn/socket-quit.
pub const SERVICE_UNIT: &str = "hyprlay";

/// Map a toggle + unit presence to the mechanism.
///
/// Starting prefers `systemctl start` when the unit is installed, else spawns
/// a sibling daemon. Stopping honours [`StopPolicy`]: [`ViaSystemctl`] becomes
/// `systemctl stop` only when the unit is the installed supervisor, otherwise
/// it falls back to the socket; [`ViaSocket`] always quits over the ctl
/// socket.
pub fn plan_action(toggle: Toggle, unit_installed: bool, stop: StopPolicy) -> Action {
    match toggle {
        Toggle::Start if unit_installed => Action::SystemctlStart,
        Toggle::Start => Action::SpawnDaemon,
        Toggle::Stop => match stop {
            // `systemctl stop` only when the unit is the installed supervisor.
            StopPolicy::ViaSystemctl if unit_installed => Action::SystemctlStop,
            _ => Action::SocketQuit,
        },
    }
}

/// Resolve and run one toggle press against the injected boundary.
/// Success stays silent (`None`) — the immediate status refresh that follows
/// is the visible feedback; a failure returns the text to surface.
pub fn execute_toggle(
    control: &dyn DaemonControl,
    toggle: Toggle,
    stop: StopPolicy,
) -> Option<String> {
    let action = plan_action(toggle, control.unit_installed(), stop);
    control.perform(action).err()
}

#[cfg(test)]
mod tests {
    use super::Action;
    use super::StopPolicy;
    use super::Toggle;
    use super::plan_action;

    /// The full decision matrix for one toggle press. `plan_action` is the
    /// single source of truth for every front (GUI, tray, CLI) and every
    /// backend (systemd, launchd, Windows startup), so no platform input is
    /// needed: the shape of the plan is decided purely by the toggle, the
    /// unit presence, and the stop policy.
    #[test]
    fn start_with_unit_installed_plans_service_start() {
        for stop in [StopPolicy::ViaSystemctl, StopPolicy::ViaSocket] {
            assert_eq!(
                plan_action(Toggle::Start, true, stop),
                Action::SystemctlStart,
            );
        }
    }

    #[test]
    fn start_without_unit_plans_a_spawn() {
        for stop in [StopPolicy::ViaSystemctl, StopPolicy::ViaSocket] {
            assert_eq!(plan_action(Toggle::Start, false, stop), Action::SpawnDaemon,);
        }
    }

    #[test]
    fn systemctl_stop_only_when_the_unit_is_the_supervisor() {
        assert_eq!(
            plan_action(Toggle::Stop, true, StopPolicy::ViaSystemctl),
            Action::SystemctlStop
        );
    }

    #[test]
    fn systemctl_stop_falls_back_to_socket_when_no_unit() {
        assert_eq!(
            plan_action(Toggle::Stop, false, StopPolicy::ViaSystemctl),
            Action::SocketQuit
        );
    }

    #[test]
    fn socket_stop_always_quits_over_the_socket() {
        for installed in [true, false] {
            assert_eq!(
                plan_action(Toggle::Stop, installed, StopPolicy::ViaSocket),
                Action::SocketQuit
            );
        }
    }
}
