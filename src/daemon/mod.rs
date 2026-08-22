//! iced adapter: wires the deep modules (discord RPC, commands, geometry,
//! state) into the layer-shell application and translates their outputs
//! into iced Tasks. Domain logic lives in the other modules, not here.
//! This is the daemon frontend (`hyprlayd` bin); the `hyprlay` launcher
//! execs into it, and all other CLI commands are served by the `hyprlay`
//! binary over the control socket. The wire vocabulary lives in
//! `hyprlay-core::ctl`; only this listener side is daemon-specific.

mod ctl_server;
mod overlay;

pub mod adapters;

use std::os::unix::process::CommandExt;
use std::sync::Arc;

use adapters::auth::OwnAppAuth;
use adapters::avatar;
use adapters::discord;
use hyprlay_core::ctl;
use hyprlay_core::domain::Command;
use hyprlay_core::domain::Effect;
use hyprlay_core::domain::Key;
use hyprlay_core::domain::MonitorTarget;
use hyprlay_core::domain::Value;
use iced::Color;
use iced::Subscription;
use iced::Task;
use iced_layershell::application;
use iced_layershell::reexport::KeyboardInteractivity;
use iced_layershell::reexport::Layer;
use iced_layershell::settings::LayerShellSettings;
use iced_layershell::settings::Settings;
use iced_layershell::settings::StartMode;
use iced_layershell::to_layer_message;
use overlay::geometry;
use overlay::state;
use overlay::state::Overlay;
use overlay::view;
use tokio::sync::oneshot;

#[to_layer_message]
#[derive(Debug)]
enum Message {
    Discord(discord::DiscordEvent),
    AvatarResult {
        user_id: String,
        data: Option<Vec<u8>>,
    },
    Ctl {
        command: String,
        reply: oneshot::Sender<String>,
    },
}

/// Daemon entry point, called by the thin `src/bin/hyprlayd.rs` main.
pub fn run() -> iced_layershell::Result {
    init_logging();

    // Single-instance guard: a second daemon exits before it creates a
    // layer surface or steals the control socket, with a visible stderr
    // error and exit code 1 — an explicit launch that cannot run is a
    // failed request (D7), while autostart `&` users are unaffected by a
    // nonzero exit of the loser. The JSON event is kept for log-based
    // diagnosis. The probe and the later listener bind are not atomic —
    // two daemons launched in the same instant can both see "free" and
    // race the bind; the loser only warns and runs without remote control,
    // which is acceptable for that pathological simultaneous launch.
    match ctl::probe_socket(&ctl::socket_path()) {
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
    // shared with the Discord subscription, so changed credentials only
    // take effect through the `restart` command. With none present,
    // detect() logs the single `credentials_missing` event and the rpc
    // task parks until a restart starts a fresh process.
    let auth = adapters::auth::detect();

    // No text input in the overlay; skip the always-on clipboard worker.
    iced_layershell::disable_clipboard();

    let size = (cfg.width, 64);
    let offset = geometry::offset(&cfg);
    let anchor = geometry::anchor(&cfg);
    let start_mode = match cfg.monitor.as_deref() {
        Some(name) => StartMode::TargetScreen(name.to_string()),
        None => StartMode::Active,
    };
    let rpc_auth = DiscordRpc(auth.map(Arc::new));

    application(
        move || {
            let mut overlay = Overlay::new(cfg.clone());
            overlay.hydrate_roster();
            (overlay, Task::none())
        },
        "hyprlay",
        update,
        view::view,
    )
    .style(|_state, _theme| iced::theme::Style {
        background_color: Color::TRANSPARENT,
        text_color: Color::WHITE,
    })
    .subscription(move |_state| subscription(&rpc_auth))
    .settings(Settings {
        layer_settings: LayerShellSettings {
            anchor,
            layer: Layer::Top,
            exclusive_zone: 0,
            size: Some(size),
            margin: offset,
            keyboard_interactivity: KeyboardInteractivity::None,
            // The overlay is pure display: every pointer event passes
            // through to whatever is below, unconditionally.
            events_transparent: true,
            start_mode,
        },
        ..Default::default()
    })
    .run()
}

/// Two outputs: machine-readable JSON wide events under
/// `$XDG_STATE_HOME/hyprlay/logs/` (per the logging guidelines), and
/// a human-friendly stream on stderr showing only our lifecycle messages and
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
/// they are fixed for the whole daemon lifetime — so hashing only the
/// stable id keeps the stream identity (and its connection) unchanged
/// across UI updates.
#[derive(Clone)]
struct DiscordRpc(Option<Arc<OwnAppAuth>>);

impl std::hash::Hash for DiscordRpc {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "discord-rpc".hash(state);
    }
}

fn subscription(auth: &DiscordRpc) -> Subscription<Message> {
    Subscription::batch([
        Subscription::run_with(auth.clone(), discord_subscription),
        Subscription::run(ctl_subscription),
    ])
}

fn discord_subscription(rpc: &DiscordRpc) -> impl futures_util::Stream<Item = Message> + use<> {
    // `use<>`: the stream owns a backend clone, so the opaque type must
    // stay 'static instead of capturing the `&rpc` borrow (edition 2024).
    use futures_util::StreamExt;
    let backend = rpc.0.clone();
    iced::stream::channel(64, move |sender| async move {
        discord::run(sender, backend).await;
    })
    .map(Message::Discord)
}

fn ctl_subscription() -> impl futures_util::Stream<Item = Message> {
    use futures_util::StreamExt;
    ctl_server::incoming().map(|req| Message::Ctl {
        command: req.command,
        reply: req.reply,
    })
}

fn update(state: &mut Overlay, message: Message) -> Task<Message> {
    match message {
        Message::Discord(ev) => match state.apply_discord(ev) {
            state::RosterChange::Changed => Task::batch([resize_task(state), avatar_tasks(state)]),
            state::RosterChange::Unchanged => Task::none(),
        },
        Message::AvatarResult { user_id, data } => {
            if let Some(data) = data {
                state.insert_avatar(user_id, data);
            }
            Task::none()
        }
        Message::Ctl { command, reply } => handle_ctl(state, command, reply),
        // Variants generated by #[to_layer_message] are sent as Tasks and
        // consumed by the layershell runtime, never re-delivered here.
        _ => Task::none(),
    }
}

/// Control-socket entry point: the command line is parsed once into a typed
/// [`Command`]; daemon-side commands are answered here, everything else goes
/// through `apply_config` and the resulting effects are translated into Tasks.
fn handle_ctl(
    state: &mut Overlay,
    command: String,
    reply: oneshot::Sender<String>,
) -> Task<Message> {
    let cmd: Command = match command.parse() {
        Ok(cmd) => cmd,
        Err(err) => {
            let _ = reply.send(err);
            return Task::none();
        }
    };

    // Daemon-side commands need live state or IO; answer them directly.
    match cmd {
        Command::Save => {
            state.config_mut().save();
            let _ = reply.send("saved".to_string());
            return Task::none();
        }
        Command::Dump => {
            let _ = reply.send(toml::to_string(state.config()).unwrap_or_default());
            return Task::none();
        }
        Command::Status => {
            let cfg = state.config();
            let corner = hyprlay_core::domain::corner_of(cfg.horizontal, cfg.vertical);
            let _ = reply.send(format!(
                "status={} channel={} participants={} position={} rtl={} visible={} anchor={} scale={} opacity={} offset=({},{}) monitor={monitor} auth={auth}",
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
            ));
            return Task::none();
        }
        Command::Help => {
            let _ = reply.send(ctl::usage());
            return Task::none();
        }
        Command::Get(key) => {
            let _ = reply.send(key.get(state.config()));
            return Task::none();
        }
        // Layer-shell surfaces bind to an output at creation, so a monitor
        // change re-creates the surface via exec-restart. Bare `set monitor`
        // cycles active -> each detected output.
        Command::Restart => {
            // The GUI sends this after writing new credentials: only a
            // fresh process re-runs detect() and picks the new backend.
            // Promise the restart only when the re-exec target exists,
            // mirroring the monitor-change guard below.
            if !can_reexec() {
                tracing::error!(
                    event = "daemon_restart_failed",
                    "aborting restart: daemon binary unavailable"
                );
                let _ =
                    reply.send("error: could not restart daemon: binary is missing".to_string());
                return Task::none();
            }
            let _ = reply.send("restarting".to_string());
            return restart_daemon();
        }
        Command::Quit => {
            // Same reply-then-act shape as restart: the oneshot wakes the
            // connection writer first, then the runtime exits cleanly (layer
            // surface dropped, ctl listener dies with the event loop, exit
            // code 0).
            let _ = reply.send(Command::QUIT_REPLY.to_string());
            return clean_shutdown();
        }
        Command::Set(Key::Monitor, value) => {
            let target = match value {
                Value::Target(target) => Some(target),
                Value::Cycle => next_monitor_target(state.config().monitor.as_deref()),
                _ => None,
            };
            let Some(target) = target else {
                let _ = reply.send("error: no monitors reported".to_string());
                return Task::none();
            };
            // An unknown output would restart into whatever fallback the
            // shell picks for it — reject instead of moving somewhere the
            // user did not ask for.
            if let MonitorTarget::Named(name) = &target {
                let known = monitor_known(name, &hyprlay_core::compositor::detect().monitors());
                if !known {
                    let _ = reply.send(format!(
                        "error: no output named {name} (try 'hyprlay monitors')"
                    ));
                    return Task::none();
                }
            }
            // Same-output requests must not cost a surface re-creation and
            // the ctl downtime that comes with it.
            if !monitor_change_restarts(state.config().monitor.as_deref(), &target) {
                let where_at = match &target {
                    MonitorTarget::Active => "the active monitor".to_string(),
                    MonitorTarget::Named(name) => name.clone(),
                };
                let _ = reply.send(format!("already on {where_at}"));
                return Task::none();
            }
            let text = match &target {
                MonitorTarget::Active => "restarting on the active monitor".to_string(),
                MonitorTarget::Named(name) => format!("restarting on {name}"),
            };
            // The reply promises a restart — promise it only while the
            // re-exec target provably exists. exec() used to fail with
            // ENOENT after this point (binary rebuilt away under a running
            // daemon), reporting success while the old surface stayed put.
            if !can_reexec() {
                tracing::error!(
                    event = "daemon_restart_failed",
                    "aborting monitor change: daemon binary unavailable"
                );
                let _ = reply.send(
                    "error: could not relocate overlay: daemon binary is missing".to_string(),
                );
                return Task::none();
            }
            state.config_mut().monitor = match target {
                MonitorTarget::Active => None,
                MonitorTarget::Named(name) => Some(name),
            };
            // Deliberate exception to should_persist/autosave: this process
            // is about to be replaced and the fresh one re-reads config.toml
            // to decide where to bind, so skipping the write would silently
            // lose the monitor choice.
            state.config_mut().save();
            let _ = reply.send(text);
            return restart_daemon();
        }
        _ => {}
    }

    // Daemon-owned persistence: with autosave on, every successful state
    // mutation is written through immediately; failed commands and
    // read-only/lifecycle commands leave disk alone. `save` stays as the
    // explicit force-write, monitor changes save in their own arm.
    let persists = hyprlay_core::domain::should_persist(&cmd, state.config().auto_save);
    let result = cmd.apply_config(state.config_mut());
    if persists && !result.reply.starts_with("error:") {
        state.config().save();
    }
    let mut tasks: Vec<Task<Message>> = Vec::new();

    for effect in result.effects {
        match effect {
            Effect::Resize => tasks.push(resize_task(state)),
            Effect::Reanchor => {
                state.reanchor();
                let anchor = geometry::anchor(state.config());
                tasks.push(Task::done(Message::AnchorChange(anchor)));
                tasks.push(Task::done(Message::MarginChange(state.offset())));
            }
            Effect::Nudge(dx, dy) => {
                state.nudge(dx, dy);
                tasks.push(Task::done(Message::MarginChange(state.offset())));
            }
        }
    }

    let _ = reply.send(result.reply);
    Task::batch(tasks)
}

/// Whether switching to `requested` needs a fresh surface. Layer-shell
/// surfaces bind to an output at creation and cannot move at runtime, so a
/// real change re-creates the whole daemon; asking for the output already
/// in effect must not churn the ctl socket and flicker the overlay.
fn monitor_change_restarts(current: Option<&str>, requested: &MonitorTarget) -> bool {
    match requested {
        // Started without a pin already: re-exec would resolve "active" to
        // whatever is focused after the restart, not move anything now.
        MonitorTarget::Active => current.is_some(),
        MonitorTarget::Named(name) => current != Some(name.as_str()),
    }
}

/// The requested output exists right now? Guards against restarting into
/// whatever fallback the shell picks for an unknown name — descriptions are
/// deliberately not accepted, only connector-style names from `monitors`.
fn monitor_known(name: &str, monitors: &[hyprlay_core::compositor::Monitor]) -> bool {
    monitors.iter().any(|m| m.name == name)
}

/// The next entry in the [active, monitor1, monitor2, ...] cycle after the
/// currently configured target; `None` when no outputs are reported.
fn next_monitor_target(current: Option<&str>) -> Option<MonitorTarget> {
    let mut options: Vec<Option<String>> = vec![None];
    options.extend(
        hyprlay_core::compositor::detect()
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

/// Match the surface size to the current roster; emits SizeChange only when
/// it actually changed.
fn resize_task(state: &mut Overlay) -> Task<Message> {
    state
        .take_size_change()
        .map(|size| Task::done(Message::SizeChange(size)))
        .unwrap_or_else(Task::none)
}

/// Kick off fetches for any participant avatars we don't have yet.
fn avatar_tasks(state: &mut Overlay) -> Task<Message> {
    let missing = state.claim_missing_avatars();
    Task::batch(missing.into_iter().map(|(user_id, hash, url)| {
        Task::perform(avatar::fetch(user_id.clone(), hash, url), move |data| {
            Message::AvatarResult {
                user_id: user_id.clone(),
                data,
            }
        })
    }))
}

/// Re-exec the daemon detached so the new process recreates its layer
/// surface (and picks up the new `monitor` config), then exit this one.
/// Replace this process image with a fresh daemon so the layer surface is
/// recreated against the new monitor. `exec()` keeps our PID, which is what
/// makes this safe under supervisors: a systemd unit (e.g. `bgrun`) tracks
/// us by cgroup, and a fork+exit here would get the replacement child torn
/// down together with the old unit's cgroup.
/// True while the on-disk image `restart_daemon` would exec into still
/// exists. A running daemon outlives rebuilds that delete its binary, and
/// exec() then fails with ENOENT — callers check here first so they can
/// answer honestly instead of promising a restart that cannot happen.
fn can_reexec() -> bool {
    std::env::current_exe()
        .and_then(|exe| std::fs::metadata(exe).map(|_| ()))
        .is_ok()
}

/// The visible second-daemon failure line (D7). Kept pure so the exact
/// wording is pinned by a test; main.rs wiring (stderr + exit 1) is
/// verified live.
fn already_running_message(path: &std::path::Path) -> String {
    format!("error: daemon already running ({})", path.display())
}

/// Clean shutdown for `quit`: the exit task makes the runtime stop its
/// event loop, which drops the layer surface and ends `main` normally with
/// exit code 0. The ctl listener lives inside a subscription stream and is
/// torn down with the loop. This is the one piece of the quit path without
/// an in-process pin — it is a single runtime call, verified by live smoke.
fn clean_shutdown() -> Task<Message> {
    iced::exit()
}

fn restart_daemon() -> Task<Message> {
    match std::env::current_exe() {
        Ok(exe) => {
            let err = std::process::Command::new(&exe).arg("daemon").exec();
            tracing::error!(
                event = "daemon_restart_failed",
                error = %err,
                "could not re-exec daemon"
            );
        }
        Err(e) => {
            tracing::error!(event = "daemon_restart_failed", error = %e, "could not resolve current_exe");
        }
    }
    // exec() never returns; reaching this line means restart failed, so keep
    // the current surface alive rather than dropping the overlay entirely.
    Task::none()
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
            },
            Monitor {
                name: "HDMI-A-1".to_string(),
                description: "AOC G2490W1G4".to_string(),
                active: false,
            },
        ];
        assert!(monitor_known("eDP-1", &monitors));
        assert!(!monitor_known("DP-9", &monitors));
        // Descriptions are not names, even when they contain the name.
        assert!(!monitor_known("AOC G2490W1G4", &monitors));
    }
}
