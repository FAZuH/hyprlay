//! Daemon Start/Stop routing shared by every front (GUI, tray, CLI).
//!
//! Fronts may only depend on `hyprlay-core`, never on each other, so the
//! toggle→action planning and the real process/socket boundary live here as
//! one source of truth. Each front keeps its own `DaemonState` / poll state
//! machine; what is shared is the *decision*: given a [`Toggle`] and whether
//! the systemd user unit is installed, which [`Action`] should run, and how
//! each action is performed by [`SystemControl`].
//!
//! The Stop policy differs per front: the GUI stops via `systemctl stop`
//! when the unit is installed, while the tray always quits over the ctl
//! socket (`quit`) so it can tear down a daemon started by any supervisor,
//! sibling, or socket-activated unit. [`StopPolicy`] carries that choice.

use std::process::Stdio;

use crate::ctl;
use crate::domain::Command;

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

/// systemd user unit name. Its presence routes Start/Stop through systemctl;
/// its absence falls back to spawn/socket-quit.
const SERVICE_UNIT: &str = "hyprlay";

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

/// Real boundary: systemctl for the unit paths, detached sibling spawn and
/// the control socket otherwise.
pub struct SystemControl;

impl DaemonControl for SystemControl {
    fn unit_installed(&self) -> bool {
        // Exit success is the whole verdict; the printed unit is noise.
        std::process::Command::new("systemctl")
            .args(["--user", "cat", SERVICE_UNIT])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn perform(&self, action: Action) -> Result<(), String> {
        match action {
            Action::SystemctlStart => systemctl("start"),
            Action::SystemctlStop => systemctl("stop"),
            Action::SpawnDaemon => spawn_sibling_daemon(),
            Action::SocketQuit => quit_via_socket(),
        }
    }
}

/// Run one systemctl subcommand on our user unit; failure text carries
/// systemctl's own stderr so the caller can explain what went wrong.
pub fn systemctl(subcommand: &str) -> Result<(), String> {
    let output = std::process::Command::new("systemctl")
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

/// Detached sibling spawn of `hyprlayd` next to this binary. A gui→cli
/// dependency would drag clap into this binary, so the resolution is kept
/// here, next to the toggle routing it serves. The generic "spawn any
/// sibling" helper (used by the tray's Open-settings action) is intentionally
/// not here — that sibling-binary surface is unified separately.
fn spawn_sibling_daemon() -> Result<(), String> {
    const DAEMON_BIN: &str = "hyprlayd";
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
    use std::os::unix::process::CommandExt;
    // Own process group + null stdio + no wait: the daemon must outlive this
    // binary and never hold its terminal. A double start is already guarded
    // daemon-side by the socket probe.
    std::process::Command::new(&path)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("error: could not start {DAEMON_BIN}: {e}"))
}

fn quit_via_socket() -> Result<(), String> {
    ctl::send_command_line(&Command::Quit.to_string())
        .map(|_| ())
        .ok_or_else(|| "error: daemon unreachable".to_string())
}
