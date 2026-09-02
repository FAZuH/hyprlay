//! Daemon frontend (`hyprlayd` bin): wires the deep modules (discord RPC,
//! commands, geometry, state) into a platform-selected surface host and
//! translates their outputs into that host's iced Tasks. Domain logic lives
//! in the other modules, not here. This module owns the daemon lifecycle
//! (logging, single-instance probe, credential detection) and the
//! command-resolution logic shared by both surface hosts; the actual iced
//! shell (layer-shell on Linux, winit on Windows/macOS/X11) lives in
//! `surface_host/`. The `hyprlay` launcher execs into this bin, and all other
//! CLI commands are served by the `hyprlay` binary over the control socket.
//! The wire vocabulary lives in `hyprlay-core::ctl`.

mod ctl_server;
mod overlay;

pub mod adapters;
pub mod surface_host;

use std::hash::Hash;
use std::process::ExitCode;
use std::sync::Arc;

use adapters::auth::OwnAppAuth;
use hyprlay_core::compositor::Monitor;
use hyprlay_core::ctl;
use hyprlay_core::domain::Command;
use hyprlay_core::domain::Effect;
use hyprlay_core::domain::Group;
use hyprlay_core::domain::Key;
use hyprlay_core::domain::MonitorTarget;
use hyprlay_core::domain::Value;
use overlay::state::Overlay;

/// Daemon entry point, called by the thin `src/bin/hyprlayd.rs` main. Runs
/// the platform-independent lifecycle (logging, single-instance guard,
/// config load, credential detection) then hands off to the platform-selected
/// surface host.
pub fn run() -> ExitCode {
    init_logging();

    // Single-instance guard: a second daemon exits before it creates a
    // surface or steals the control socket, with a visible stderr error and
    // exit code 1 — an explicit launch that cannot run is a failed request
    // (D7), while autostart `&` users are unaffected by a nonzero exit of
    // the loser. The JSON event is kept for log-based diagnosis. The probe
    // and the later listener bind are not atomic — two daemons launched in
    // the same instant can both see "free" and race the bind; the loser only
    // warns and runs without remote control, which is acceptable for that
    // pathological simultaneous launch.
    match ctl::probe_socket(&crate::platform::ipc::control::Control, &ctl::socket_path()) {
        ctl::SocketProbe::AlreadyRunning => {
            tracing::info!(
                event = "daemon_already_running",
                "control socket owned by a live daemon; exiting"
            );
            eprintln!("{}", already_running_message(&ctl::socket_path()));
            std::process::exit(1);
        }
        ctl::SocketProbe::StaleRemoved => {
            tracing::info!(
                event = "daemon_stale_socket_removed",
                "removed a stale control socket from an earlier run"
            );
        }
        ctl::SocketProbe::Free => {}
    }

    let cfg = hyprlay_core::config::load();

    // Credentials are a per-process choice: detected exactly once here and
    // shared with the Discord subscription, so changed credentials only take
    // effect through the `restart` command. With none present, detect() logs
    // the single `credentials_missing` event and the rpc task parks until a
    // restart starts a fresh process.
    let auth = adapters::auth::detect();

    surface_host::run(cfg, auth)
}

/// Two outputs: machine-readable JSON wide events under
/// `$XDG_STATE_HOME/hyprlay/logs/` (per the logging guidelines), and a
/// human-friendly stream on stderr showing only our lifecycle messages and
/// real errors — library noise (wgpu, layershellev) stays in the file.
fn init_logging() {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let log_dir = dirs::state_dir()
        .map(|d| d.join("hyprlay"))
        .or_else(|| dirs::data_local_dir().map(|d| d.join("hyprlay")))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("logs");
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        eprintln!(
            "warning: could not create log dir {}: {e}",
            log_dir.display()
        );
    }
    let appender = tracing_appender::rolling::daily(&log_dir, "hyprlay.log");

    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(appender)
        .with_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()));

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .compact()
        .with_target(false)
        .without_time()
        .with_filter(EnvFilter::new("error,discord_overlay=info"));

    tracing_subscriber::registry()
        .with(file_layer)
        .with(stderr_layer)
        .init();
    eprintln!("hyprlay: logging to {}/hyprlay.log.*", log_dir.display());
}

/// Subscription payload for the Discord RPC stream. iced identifies
/// subscriptions by hash and the credentials carry no meaningful hash, but
/// they are fixed for the whole daemon lifetime — so hashing only the stable
/// id keeps the stream identity (and its connection) unchanged across UI
/// updates. Shared by both surface hosts; moving it out of the layer-shell
/// module keeps the subscription key identical between arms.
#[derive(Clone)]
pub(crate) struct DiscordRpc(pub(crate) Option<Arc<OwnAppAuth>>);

impl Hash for DiscordRpc {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "discord-rpc".hash(state);
    }
}

/// A lifecycle action a command requests, decoupled from any shell task so
/// both surface hosts interpret it the same way (`restart` is a re-exec and
/// `quit` is a clean shutdown).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lifecycle {
    Restart,
    Quit,
}

/// The outcome of resolving a control command against live daemon state,
/// before any shell tasks are produced: the reply to send back, the domain
/// effects to translate, and any lifecycle action to run.
pub(crate) struct CommandOutcome {
    pub reply: String,
    pub effects: Vec<Effect>,
    pub lifecycle: Option<Lifecycle>,
}

/// Whether there is a monitor-aware placement for the overlay right now.
/// Shared by both arms' hover paths.
pub(crate) fn monitor_for_overlay(cfg: &hyprlay_core::config::Config) -> Option<Monitor> {
    let monitors = crate::platform::compositor::detect().monitors();
    crate::daemon::overlay::geometry::pick_monitor(&monitors, cfg.monitor.as_deref()).cloned()
}

pub(crate) fn hover_poll_enabled(state: &Overlay) -> bool {
    state.config().dim_on_hover
        && state.config().visible
        && !state.displayed().is_empty()
        && state.status() == hyprlay_core::domain::ConnectionStatus::Connected
}

/// Control-socket command resolution. The command line is parsed once into a
/// typed [`Command`]; daemon-side commands are answered here, everything else
/// goes through `apply_config`. Returns the reply, the domain effects, and
/// any lifecycle action — each surface host turns those into its own Tasks.
/// This is the single source of truth for what each command *means*; only the
/// shell-specific emission of the effects differs between arms.
pub(crate) fn resolve_command(state: &mut Overlay, cmd: Command) -> CommandOutcome {
    // Daemon-side commands need live state or IO; answer them directly.
    match cmd {
        Command::Save => {
            state.config_mut().save();
            return CommandOutcome {
                reply: "saved".to_string(),
                effects: Vec::new(),
                lifecycle: None,
            };
        }
        Command::Dump => {
            return CommandOutcome {
                reply: toml::to_string(state.config()).unwrap_or_default(),
                effects: Vec::new(),
                lifecycle: None,
            };
        }
        Command::Status => {
            let cfg = state.config();
            let corner = hyprlay_core::domain::corner_of(cfg.horizontal, cfg.vertical);
            return CommandOutcome {
                reply: format!(
                    "status={} channel={} participants={} position={} rtl={} visible={} anchor={} scale={} opacity={} offset=({},{}) monitor={monitor} auth={auth} show-on-fullscreen={show_on} dim-on-hover={dim_on} hover-opacity={hover}",
                    state.status(),
                    state.channel_name().unwrap_or("-"),
                    state.displayed().len(),
                    hyprlay_core::domain::corner_word(corner),
                    if cfg.rtl { "on" } else { "off" },
                    if cfg.visible { "on" } else { "off" },
                    cfg.anchor,
                    cfg.scale,
                    cfg.opacity,
                    cfg.offset_x,
                    cfg.offset_y,
                    monitor = cfg.monitor.as_deref().unwrap_or("active"),
                    auth = state.auth_label(),
                    show_on = if cfg.show_on_fullscreen { "on" } else { "off" },
                    dim_on = if cfg.dim_on_hover { "on" } else { "off" },
                    hover = cfg.hover_opacity,
                ),
                effects: Vec::new(),
                lifecycle: None,
            };
        }
        Command::Help => {
            return CommandOutcome {
                reply: ctl::usage(),
                effects: Vec::new(),
                lifecycle: None,
            };
        }
        Command::Get(key) => {
            return CommandOutcome {
                reply: key.get(state.config()),
                effects: Vec::new(),
                lifecycle: None,
            };
        }
        // Layer-shell surfaces bind to an output at creation, so a monitor
        // change re-creates the surface via exec-restart. Bare `set monitor`
        // cycles active -> each detected output.
        Command::Restart => {
            // The GUI sends this after writing new credentials: only a fresh
            // process re-runs detect() and picks the new backend. Promise the
            // restart only when the re-exec target exists, mirroring the
            // monitor-change guard below.
            if !can_reexec() {
                tracing::error!(
                    event = "daemon_restart_failed",
                    "aborting restart: daemon binary unavailable"
                );
                return CommandOutcome {
                    reply: "error: could not restart daemon: binary is missing".to_string(),
                    effects: Vec::new(),
                    lifecycle: None,
                };
            }
            return CommandOutcome {
                reply: "restarting".to_string(),
                effects: Vec::new(),
                lifecycle: Some(Lifecycle::Restart),
            };
        }
        Command::Quit => {
            // Same reply-then-act shape as restart: the oneshot wakes the
            // connection writer first, then the runtime exits cleanly
            // (surface dropped, ctl listener dies with the event loop, exit
            // code 0).
            return CommandOutcome {
                reply: Command::QUIT_REPLY.to_string(),
                effects: Vec::new(),
                lifecycle: Some(Lifecycle::Quit),
            };
        }
        Command::Set(Key::Monitor, value) => {
            let target = match value {
                Value::Target(target) => Some(target),
                Value::Cycle => next_monitor_target(state.config().monitor.as_deref()),
                _ => None,
            };
            let Some(target) = target else {
                return CommandOutcome {
                    reply: "error: no monitors reported".to_string(),
                    effects: Vec::new(),
                    lifecycle: None,
                };
            };
            // An unknown output would restart into whatever fallback the
            // shell picks for it — reject instead of moving somewhere the
            // user did not ask for.
            if let MonitorTarget::Named(name) = &target {
                let known = monitor_known(name, &crate::platform::compositor::detect().monitors());
                if !known {
                    return CommandOutcome {
                        reply: format!("error: no output named {name} (try 'hyprlay monitors')"),
                        effects: Vec::new(),
                        lifecycle: None,
                    };
                }
            }
            // Same-output requests must not cost a surface re-creation and
            // the ctl downtime that comes with it.
            if !monitor_change_restarts(state.config().monitor.as_deref(), &target) {
                let where_at = match &target {
                    MonitorTarget::Active => "the active monitor".to_string(),
                    MonitorTarget::Named(name) => name.clone(),
                };
                return CommandOutcome {
                    reply: format!("already on {where_at}"),
                    effects: Vec::new(),
                    lifecycle: None,
                };
            }
            let text = match &target {
                MonitorTarget::Active => "restarting on the active monitor".to_string(),
                MonitorTarget::Named(name) => format!("restarting on {name}"),
            };
            // The reply promises a restart — promise it only while the
            // re-exec target provably exists. exec() used to fail with ENOENT
            // after this point (binary rebuilt away under a running daemon),
            // reporting success while the old surface stayed put.
            if !can_reexec() {
                tracing::error!(
                    event = "daemon_restart_failed",
                    "aborting monitor change: daemon binary unavailable"
                );
                return CommandOutcome {
                    reply: "error: could not relocate overlay: daemon binary is missing"
                        .to_string(),
                    effects: Vec::new(),
                    lifecycle: None,
                };
            }
            state.config_mut().monitor = match target {
                MonitorTarget::Active => None,
                MonitorTarget::Named(name) => Some(name),
            };
            // Deliberate exception to should_persist/autosave: this process is
            // about to be replaced and the fresh one re-reads config.toml to
            // decide where to bind, so skipping the write would silently lose
            // the monitor choice.
            state.config_mut().save();
            return CommandOutcome {
                reply: text,
                effects: Vec::new(),
                lifecycle: Some(Lifecycle::Restart),
            };
        }
        Command::Set(Key::ShowOnFullscreen, value) => {
            let requested = match value {
                Value::Flag(b) => b,
                Value::Cycle => !state.config().show_on_fullscreen,
                _ => {
                    return CommandOutcome {
                        reply: "error: show-on-fullscreen <on|off>".to_string(),
                        effects: Vec::new(),
                        lifecycle: None,
                    };
                }
            };
            if !show_on_fullscreen_change_restarts(state.config().show_on_fullscreen, requested) {
                return CommandOutcome {
                    reply: format!(
                        "already show-on-fullscreen={}",
                        if requested { "on" } else { "off" }
                    ),
                    effects: Vec::new(),
                    lifecycle: None,
                };
            }
            if !can_reexec() {
                tracing::error!(
                    event = "daemon_restart_failed",
                    "aborting show-on-fullscreen change: daemon binary unavailable"
                );
                return CommandOutcome {
                    reply: "error: could not change overlay layer: daemon binary is missing"
                        .to_string(),
                    effects: Vec::new(),
                    lifecycle: None,
                };
            }
            state.config_mut().show_on_fullscreen = requested;
            state.config_mut().save();
            return CommandOutcome {
                reply: format!(
                    "restarting (show-on-fullscreen={})",
                    if requested { "on" } else { "off" }
                ),
                effects: Vec::new(),
                lifecycle: Some(Lifecycle::Restart),
            };
        }
        Command::ResetAll if reset_needs_restart(state, &cmd) => {
            if !can_reexec() {
                tracing::error!(
                    event = "daemon_restart_failed",
                    "aborting reset: daemon binary unavailable"
                );
                return CommandOutcome {
                    reply: "error: could not reset overlay: daemon binary is missing".to_string(),
                    effects: Vec::new(),
                    lifecycle: None,
                };
            }
            let requested = hyprlay_core::config::Config::default().show_on_fullscreen;
            let monitor = state.config().monitor.clone();
            *state.config_mut() = hyprlay_core::config::Config::default();
            state.config_mut().monitor = monitor;
            state.config_mut().save();
            return CommandOutcome {
                reply: format!(
                    "restarting (reset show-on-fullscreen={})",
                    if requested { "on" } else { "off" }
                ),
                effects: Vec::new(),
                lifecycle: Some(Lifecycle::Restart),
            };
        }
        Command::ResetGroup(group)
            if group == Group::Layout && reset_needs_restart(state, &cmd) =>
        {
            if !can_reexec() {
                tracing::error!(
                    event = "daemon_restart_failed",
                    "aborting reset layout: daemon binary unavailable"
                );
                return CommandOutcome {
                    reply: "error: could not reset layout: daemon binary is missing".to_string(),
                    effects: Vec::new(),
                    lifecycle: None,
                };
            }
            let requested = hyprlay_core::config::Config::default().show_on_fullscreen;
            let defaults = hyprlay_core::config::Config::default();
            for key in Key::ALL {
                if key == Key::Monitor || key.group() != Group::Layout {
                    continue;
                }
                if key == Key::ShowOnFullscreen {
                    state.config_mut().show_on_fullscreen = defaults.show_on_fullscreen;
                    continue;
                }
                key.apply(state.config_mut(), key.value_of(&defaults));
            }
            state.config_mut().save();
            return CommandOutcome {
                reply: format!(
                    "restarting (reset layout show-on-fullscreen={})",
                    if requested { "on" } else { "off" }
                ),
                effects: Vec::new(),
                lifecycle: Some(Lifecycle::Restart),
            };
        }
        _ => {}
    }

    // Daemon-owned persistence: with autosave on, every successful state
    // mutation is written through immediately; failed commands and
    // read-only/lifecycle commands leave disk alone. `save` stays as the
    // explicit force-write, monitor changes save in their own arm.
    let persists = hyprlay_core::domain::should_persist(&cmd, state.config().auto_save);
    let result = cmd.apply_config(state.config_mut());
    if !hover_poll_enabled(state) && state.is_hovered() {
        state.set_hovered(false);
    }
    if persists && !result.reply.starts_with("error:") {
        state.config().save();
    }
    CommandOutcome {
        reply: result.reply,
        effects: result.effects,
        lifecycle: None,
    }
}

/// Whether switching to `requested` needs a fresh surface. Layer-shell
/// surfaces bind to an output at creation and cannot move at runtime, so a
/// real change re-creates the whole daemon; asking for the output already in
/// effect must not churn the ctl socket and flicker the overlay. On the winit
/// arm, the window can move, but the same policy keeps the arms' UX
/// consistent (a monitor change re-runs the daemon process).
pub(crate) fn monitor_change_restarts(current: Option<&str>, requested: &MonitorTarget) -> bool {
    match requested {
        // Started without a pin already: re-exec would resolve "active" to
        // whatever is focused after the restart, not move anything now.
        MonitorTarget::Active => current.is_some(),
        MonitorTarget::Named(name) => current != Some(name.as_str()),
    }
}

pub(crate) fn show_on_fullscreen_change_restarts(current: bool, requested: bool) -> bool {
    current != requested
}

pub(crate) fn reset_needs_restart(state: &Overlay, cmd: &Command) -> bool {
    match cmd {
        Command::ResetAll => show_on_fullscreen_change_restarts(
            state.config().show_on_fullscreen,
            hyprlay_core::config::Config::default().show_on_fullscreen,
        ),
        Command::ResetGroup(group) if *group == Group::Layout => {
            show_on_fullscreen_change_restarts(
                state.config().show_on_fullscreen,
                hyprlay_core::config::Config::default().show_on_fullscreen,
            )
        }
        _ => false,
    }
}

/// The requested output exists right now? Guards against restarting into
/// whatever fallback the shell picks for an unknown name — descriptions are
/// deliberately not accepted, only connector-style names from `monitors`.
pub(crate) fn monitor_known(name: &str, monitors: &[Monitor]) -> bool {
    monitors.iter().any(|m| m.name == name)
}

/// The next entry in the [active, monitor1, monitor2, ...] cycle after the
/// currently configured target; `None` when no outputs are reported.
pub(crate) fn next_monitor_target(current: Option<&str>) -> Option<MonitorTarget> {
    let mut options: Vec<Option<String>> = vec![None];
    options.extend(
        crate::platform::compositor::detect()
            .monitors()
            .into_iter()
            .map(|m| Some(m.name)),
    );
    if options.len() < 2 {
        return None;
    }
    let current_owned = current.map(str::to_string);
    let index = options
        .iter()
        .position(|o| *o == current_owned)
        .unwrap_or(0);
    match options[(index + 1) % options.len()].clone() {
        None => Some(MonitorTarget::Active),
        Some(name) => Some(MonitorTarget::Named(name)),
    }
}

/// True while the on-disk image a restart would re-exec into still exists.
/// A running daemon outlives rebuilds that delete its binary, and exec() then
/// fails with ENOENT — callers check here first so they can answer honestly
/// instead of promising a restart that cannot happen. (On the winit arm the
/// restart is a spawn+exit rather than an exec, but the guard is the same.)
fn can_reexec() -> bool {
    std::env::current_exe()
        .and_then(|exe| std::fs::metadata(exe).map(|_| ()))
        .is_ok()
}

/// The visible second-daemon failure line (D7). Kept pure so the exact
/// wording is pinned by a test; main.rs wiring (stderr + exit 1) is verified
/// live.
fn already_running_message(path: &std::path::Path) -> String {
    format!("error: daemon already running ({})", path.display())
}

#[cfg(test)]
mod tests {
    use hyprlay_core::compositor::Monitor;

    use super::*;

    #[test]
    fn second_daemon_error_names_the_socket_path() {
        // D7: a second daemon must fail visibly on stderr — naming the
        // socket it lost the race for — instead of vanishing into the
        // JSON log.
        assert_eq!(
            already_running_message(std::path::Path::new("/run/user/1000/hyprlay.sock")),
            "error: daemon already running (/run/user/1000/hyprlay.sock)"
        );
    }

    #[test]
    fn monitor_change_to_the_same_output_is_a_noop() {
        // The daemon only runs with one pinned-or-active choice per process,
        // so a matching name means the surface already lives there.
        assert!(!monitor_change_restarts(
            Some("HDMI-A-1"),
            &MonitorTarget::Named("HDMI-A-1".to_string())
        ));
    }

    #[test]
    fn monitor_change_to_a_different_output_restarts() {
        assert!(monitor_change_restarts(
            Some("eDP-1"),
            &MonitorTarget::Named("HDMI-A-1".to_string())
        ));
    }

    #[test]
    fn staying_in_active_mode_is_a_noop_but_leaving_it_restarts() {
        // Active-mode is resolved once at startup; re-execing for the same
        // policy would just pick whatever is focused later, not "move".
        assert!(!monitor_change_restarts(None, &MonitorTarget::Active));
        assert!(monitor_change_restarts(
            Some("eDP-1"),
            &MonitorTarget::Active
        ));
    }

    #[test]
    fn monitor_names_must_match_an_existing_output() {
        let monitors = vec![
            Monitor {
                name: "eDP-1".to_string(),
                description: "laptop panel".to_string(),
                active: true,
                ..Default::default()
            },
            Monitor {
                name: "HDMI-A-1".to_string(),
                description: "AOC G2490W1G4".to_string(),
                active: false,
                ..Default::default()
            },
        ];
        assert!(monitor_known("eDP-1", &monitors));
        assert!(!monitor_known("DP-9", &monitors));
        // Descriptions are not names, even when they contain the name.
        assert!(!monitor_known("AOC G2490W1G4", &monitors));
    }

    #[test]
    fn show_on_fullscreen_change_restarts_only_when_flag_flips() {
        assert!(!show_on_fullscreen_change_restarts(true, true));
        assert!(!show_on_fullscreen_change_restarts(false, false));
        assert!(show_on_fullscreen_change_restarts(true, false));
        assert!(show_on_fullscreen_change_restarts(false, true));
    }

    fn overlay_with(
        dim: bool,
        visible: bool,
        status: hyprlay_core::domain::ConnectionStatus,
        users: Vec<crate::daemon::adapters::discord::Participant>,
    ) -> Overlay {
        let cfg = hyprlay_core::config::Config {
            dim_on_hover: dim,
            visible,
            ..hyprlay_core::config::Config::default()
        };
        let mut o = Overlay::new(cfg);
        // Inject users directly and set status
        o.apply_discord(crate::daemon::adapters::discord::DiscordEvent::Status(
            status,
        ));
        o.apply_discord(crate::daemon::adapters::discord::DiscordEvent::Participants(users));
        o
    }

    fn participant(id: &str) -> crate::daemon::adapters::discord::Participant {
        crate::daemon::adapters::discord::Participant {
            id: id.to_string(),
            name: id.to_string(),
            avatar_hash: None,
            speaking: false,
            self_mute: false,
            self_deaf: false,
            server_mute: false,
            server_deaf: false,
        }
    }

    #[test]
    fn hover_poll_enabled_only_when_all_guards_pass() {
        use hyprlay_core::domain::ConnectionStatus;
        let empty: Vec<crate::daemon::adapters::discord::Participant> = vec![];
        let non_empty = vec![participant("1")];
        // off → never
        assert!(!hover_poll_enabled(&overlay_with(
            false,
            true,
            ConnectionStatus::Connected,
            non_empty.clone()
        )));
        // invisible → never
        assert!(!hover_poll_enabled(&overlay_with(
            true,
            false,
            ConnectionStatus::Connected,
            non_empty.clone()
        )));
        // empty roster → never
        assert!(!hover_poll_enabled(&overlay_with(
            true,
            true,
            ConnectionStatus::Connected,
            empty
        )));
        // disconnected → never
        assert!(!hover_poll_enabled(&overlay_with(
            true,
            true,
            ConnectionStatus::Disconnected,
            non_empty.clone()
        )));
        // all true → enabled
        assert!(hover_poll_enabled(&overlay_with(
            true,
            true,
            ConnectionStatus::Connected,
            non_empty
        )));
    }

    #[test]
    fn reset_needs_restart_only_when_show_on_fullscreen_would_flip() {
        let o = Overlay::new(hyprlay_core::config::Config {
            show_on_fullscreen: false,
            ..hyprlay_core::config::Config::default()
        });
        assert!(reset_needs_restart(&o, &Command::ResetAll));
        assert!(reset_needs_restart(&o, &Command::ResetGroup(Group::Layout)));
        assert!(!reset_needs_restart(
            &o,
            &Command::ResetGroup(Group::Opacity)
        ));
        // When already default, no restart
        let o2 = Overlay::new(hyprlay_core::config::Config::default());
        assert!(!reset_needs_restart(&o2, &Command::ResetAll));
        assert!(!reset_needs_restart(
            &o2,
            &Command::ResetGroup(Group::Layout)
        ));
    }

    #[test]
    fn hover_cursor_is_noop_when_disconnected_even_if_hovered() {
        use hyprlay_core::domain::ConnectionStatus;
        let mut o = overlay_with(
            true,
            true,
            ConnectionStatus::Connected,
            vec![participant("1")],
        );
        o.set_hovered(true);
        assert!(o.is_hovered());
        // Simulate HoverCursor with status Disconnected
        o.apply_discord(crate::daemon::adapters::discord::DiscordEvent::Status(
            ConnectionStatus::Disconnected,
        ));
        // Directly test the guard used in update: when disconnected, hover should be cleared
        if !hover_poll_enabled(&o) && o.is_hovered() {
            o.set_hovered(false);
        }
        assert!(!o.is_hovered());
    }
}
