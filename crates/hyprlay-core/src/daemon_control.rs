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

use crate::bins::DAEMON_BIN;

/// Why a service-control operation failed. Part of the [`DaemonControl`] /
/// [`ServiceManager`] port contract: the backends in the host package
/// construct it, fronts render it through `Display`.
///
/// The `Display` strings are the observable wording — toggle failures land
/// in the GUI/tray status line and install failures in the CLI's stderr —
/// so each variant transcribes its old `format!` string byte-for-byte, and
/// the per-variant pins below lock that transcription. Failure *identity*
/// (which operation, which path, which command) is typed; only payloads
/// that were already strings (subcommand words, command stderr) stay
/// strings.
///
/// Deliberately **not** exhaustive-by-attribute at the variant level: the
/// adapters must construct every variant from outside this crate. The
/// enum-level `#[non_exhaustive]` only forces wildcard matches on future
/// consumers, leaving construction open.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServiceError {
    /// `systemctl --user <subcommand>` could not be spawned at all:
    /// `error: could not run systemctl: <io>`.
    #[error("error: could not run systemctl: {source}")]
    SystemctlNotRun {
        #[source]
        source: std::io::Error,
    },
    /// `systemctl --user <subcommand> hyprlay` exited non-zero; the detail
    /// is systemctl's trimmed stderr, or `exit <code>` when it printed
    /// nothing: `error: systemctl <subcommand> failed: <detail>`.
    #[error("error: systemctl {subcommand} failed: {detail}")]
    SystemctlFailed { subcommand: String, detail: String },
    /// A service command could not be spawned: `<command>: could not run
    /// <program>: <io>`.
    #[error("{command}: could not run {program}: {source}")]
    CommandNotRun {
        command: String,
        program: &'static str,
        #[source]
        source: std::io::Error,
    },
    /// A service command exited non-zero; the detail is its trimmed stderr,
    /// or `exited with <status>` when it printed nothing: `<command>:
    /// <detail>`.
    #[error("{command}: {detail}")]
    CommandFailed { command: String, detail: String },
    /// A `systemctl --user <step>` step of an install failed after the
    /// files were written: `systemctl --user <step> failed: <cause>`.
    #[error("systemctl --user {step} failed: {source}")]
    SystemctlStepFailed {
        step: &'static str,
        #[source]
        source: Box<ServiceError>,
    },
    /// A `launchctl <step>` step of the start flow failed:
    /// `error: launchctl <step> failed: <cause>`.
    #[error("error: launchctl {step} failed: {source}")]
    LaunchctlStepFailed {
        step: &'static str,
        #[source]
        source: Box<ServiceError>,
    },
    /// A `launchctl <step>` step of an install failed:
    /// `launchctl <step> failed: <cause>`.
    #[error("launchctl {step} failed: {source}")]
    LaunchctlInstallStepFailed {
        step: &'static str,
        #[source]
        source: Box<ServiceError>,
    },
    /// `std::env::current_exe()` failed: `error: could not locate the
    /// running hyprlay binary: <io>`.
    #[error("error: could not locate the running hyprlay binary: {source}")]
    LocateExe {
        #[source]
        source: std::io::Error,
    },
    /// The running binary's path has no parent directory: `error: could not
    /// find the directory of <exe>`.
    #[error("error: could not find the directory of {exe}")]
    NoExeParent { exe: std::path::PathBuf },
    /// The sibling daemon binary is missing from the install directory:
    /// `error: hyprlayd not found next to the running hyprlay binary
    /// (expected <path>)\nthe hyprlay binaries must be installed together`.
    #[error(
        "error: {} not found next to the running hyprlay binary (expected {path})\nthe hyprlay binaries must be installed together",
        DAEMON_BIN
    )]
    DaemonMissing { path: std::path::PathBuf },
    /// Spawning the detached sibling daemon failed: `error: could not start
    /// hyprlayd: <io>`.
    #[error("error: could not start {}: {source}", DAEMON_BIN)]
    SpawnDaemon {
        #[source]
        source: std::io::Error,
    },
    /// The ctl-socket quit got no answer: `error: daemon unreachable`.
    #[error("error: daemon unreachable")]
    DaemonUnreachable,
    /// A subcommand this backend does not implement (only reachable on
    /// macOS/Windows, whose backends drive a subset of the port):
    /// `error: unsupported <backend> subcommand: <subcommand>`.
    #[error("error: unsupported {backend} subcommand: {subcommand}")]
    UnsupportedSubcommand {
        backend: &'static str,
        subcommand: String,
    },
    /// A platform directory could not be resolved (the LaunchAgents dir on
    /// macOS, the Startup folder on Windows): `error: could not resolve the
    /// <what>`.
    #[error("error: could not resolve the {what}")]
    ResolveDir { what: &'static str },
    /// The LaunchAgent plist path is not valid UTF-8, so it cannot ride in
    /// a `launchctl` argument vector: `error: plist path is not UTF-8`.
    #[error("error: plist path is not UTF-8")]
    NonUtf8PlistPath,
    /// A required sibling binary is missing from the install directory, so
    /// the install aborts before writing anything: `error: missing binaries
    /// for install: <names>\nthe hyprlay binaries must be installed
    /// together`.
    #[error(
        "error: missing binaries for install: {names}\nthe hyprlay binaries must be installed together"
    )]
    MissingInstallBins { names: String },
    /// Creating a service-config file's parent directory failed:
    /// `could not create <path>: <io>`.
    #[error("could not create {path}: {source}")]
    CreateDirFailed {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Writing a service-config file failed: `could not write <path>: <io>`.
    #[error("could not write {path}: {source}")]
    WriteFileFailed {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Removing a service-config file failed: `could not remove <path>:
    /// <io>`.
    #[error("could not remove {path}: {source}")]
    RemoveFileFailed {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The default install/uninstall of an OS that cannot host the hyprlay
    /// service: `install is not supported on this platform` /
    /// `uninstall is not supported on this platform`.
    #[error("{operation} is not supported on this platform")]
    UnsupportedOperation { operation: &'static str },
}

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
    fn perform(&self, action: Action) -> Result<(), ServiceError>;
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
    fn systemctl(&self, subcommand: &str) -> Result<(), ServiceError>;
    /// Detached spawn of the sibling daemon binary.
    fn spawn_daemon(&self) -> Result<(), ServiceError>;
    /// Quit the running daemon over the control socket.
    fn quit_via_socket(&self) -> Result<(), ServiceError>;
    /// Install the autostart service config next to `exe_dir` and register
    /// it; `start` controls whether it begins running immediately. Returns
    /// one human-readable line per step.
    fn install(
        &self,
        exe_dir: &Path,
        config_base: &Path,
        data_base: &Path,
        start: bool,
    ) -> Result<Vec<String>, ServiceError> {
        let _ = (exe_dir, config_base, data_base, start);
        Err(ServiceError::UnsupportedOperation {
            operation: "install",
        })
    }
    /// Uninstall the autostart service config and deregister it. Returns one
    /// human-readable line per step.
    fn uninstall(&self, config_base: &Path, data_base: &Path) -> Result<Vec<String>, ServiceError> {
        let _ = (config_base, data_base);
        Err(ServiceError::UnsupportedOperation {
            operation: "uninstall",
        })
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
    control.perform(action).err().map(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::Action;
    use super::ServiceError;
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

    // -------------------------------------------------------------------
    // ServiceError characterization pins. Each row locks one variant's
    // Display to the exact string the pre-typed `format!` sites produced;
    // rewording any of them is a breaking change to the surfaces that print
    // service errors (GUI/tray status lines, the CLI's stderr).
    // -------------------------------------------------------------------

    fn pinned_io() -> std::io::Error {
        std::io::Error::other("disk on strike")
    }

    fn pinned_error() -> ServiceError {
        ServiceError::CommandFailed {
            command: "daemon-reload".into(),
            detail: "unit not found".into(),
        }
    }

    #[test]
    fn service_error_display_reproduces_the_pinned_wording() {
        use super::ServiceError as E;
        let cases: [(E, &str); 21] = [
            (
                E::SystemctlNotRun {
                    source: pinned_io(),
                },
                "error: could not run systemctl: disk on strike",
            ),
            (
                E::SystemctlFailed {
                    subcommand: "stop".into(),
                    detail: "unit not loaded".into(),
                },
                "error: systemctl stop failed: unit not loaded",
            ),
            (
                E::CommandNotRun {
                    command: "daemon-reload".into(),
                    program: "systemctl",
                    source: pinned_io(),
                },
                "daemon-reload: could not run systemctl: disk on strike",
            ),
            (
                E::CommandFailed {
                    command: "bootout gui/1000/hyprlay.user".into(),
                    detail: "exited with 3".into(),
                },
                "bootout gui/1000/hyprlay.user: exited with 3",
            ),
            (
                E::SystemctlStepFailed {
                    step: "daemon-reload",
                    source: Box::new(pinned_error()),
                },
                "systemctl --user daemon-reload failed: daemon-reload: unit not found",
            ),
            (
                E::LaunchctlStepFailed {
                    step: "bootstrap",
                    source: Box::new(pinned_error()),
                },
                "error: launchctl bootstrap failed: daemon-reload: unit not found",
            ),
            (
                E::LaunchctlInstallStepFailed {
                    step: "enable",
                    source: Box::new(pinned_error()),
                },
                "launchctl enable failed: daemon-reload: unit not found",
            ),
            (
                E::LocateExe {
                    source: pinned_io(),
                },
                "error: could not locate the running hyprlay binary: disk on strike",
            ),
            (
                E::NoExeParent {
                    exe: "/proc/self/exe".into(),
                },
                "error: could not find the directory of /proc/self/exe",
            ),
            (
                E::DaemonMissing {
                    path: "/opt/hyprlay/hyprlayd".into(),
                },
                "error: hyprlayd not found next to the running hyprlay binary \
                 (expected /opt/hyprlay/hyprlayd)\nthe hyprlay binaries must be installed together",
            ),
            (
                E::SpawnDaemon {
                    source: pinned_io(),
                },
                "error: could not start hyprlayd: disk on strike",
            ),
            (E::DaemonUnreachable, "error: daemon unreachable"),
            (
                E::UnsupportedSubcommand {
                    backend: "Windows startup",
                    subcommand: "reload".into(),
                },
                "error: unsupported Windows startup subcommand: reload",
            ),
            (
                E::ResolveDir {
                    what: "LaunchAgents dir",
                },
                "error: could not resolve the LaunchAgents dir",
            ),
            (E::NonUtf8PlistPath, "error: plist path is not UTF-8"),
            (
                E::MissingInstallBins {
                    names: "hyprlayd".into(),
                },
                "error: missing binaries for install: hyprlayd\nthe hyprlay binaries must be installed together",
            ),
            (
                E::CreateDirFailed {
                    path: "/home/u/.config/hyprlay/systemd/user".into(),
                    source: pinned_io(),
                },
                "could not create /home/u/.config/hyprlay/systemd/user: disk on strike",
            ),
            (
                E::WriteFileFailed {
                    path: "/home/u/.config/hyprlay/systemd/user/hyprlay.service".into(),
                    source: pinned_io(),
                },
                "could not write /home/u/.config/hyprlay/systemd/user/hyprlay.service: disk on strike",
            ),
            (
                E::RemoveFileFailed {
                    path: "/home/u/.local/share/applications/hyprlay.desktop".into(),
                    source: pinned_io(),
                },
                "could not remove /home/u/.local/share/applications/hyprlay.desktop: disk on strike",
            ),
            (
                E::UnsupportedOperation {
                    operation: "install",
                },
                "install is not supported on this platform",
            ),
            (
                E::UnsupportedOperation {
                    operation: "uninstall",
                },
                "uninstall is not supported on this platform",
            ),
        ];
        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected, "{error:?}");
        }
    }

    #[test]
    fn io_failures_keep_the_io_error_as_their_source() {
        let inner = pinned_io();
        let error = ServiceError::WriteFileFailed {
            path: "/tmp/hyprlay.service".into(),
            source: inner,
        };
        let source = error
            .source()
            .expect("the io::Error rides along as the source");
        assert!(source.downcast_ref::<std::io::Error>().is_some());
    }

    #[test]
    fn nested_step_failures_chain_through_their_source() {
        let inner = pinned_error();
        let error = ServiceError::SystemctlStepFailed {
            step: "daemon-reload",
            source: Box::new(inner),
        };
        let source = error
            .source()
            .expect("the wrapped service failure is the source");
        assert_eq!(
            source.to_string(),
            "daemon-reload: unit not found",
            "the cause chain ends at the raw command failure"
        );
    }
}
