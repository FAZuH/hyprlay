//! Linux/Wayland surface host: the existing `iced_layershell` shell. Carved
//! out of `daemon/mod.rs` into its own arm; behaviour is byte-identical —
//! same anchor vocabulary, layer, `StartMode`, keyboard-interactivity and
//! transparent-events settings. This module only runs on Linux, where
//! iced_layershell (and iced's `wayland` feature) are available.

use std::process::ExitCode;
use std::sync::Arc;

use hyprlay_core::config::Config;
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
use tokio::sync::oneshot;

use crate::daemon::DiscordRpc;
use crate::daemon::Lifecycle;
use crate::daemon::adapters::auth::OwnAppAuth;
use crate::daemon::adapters::avatar;
use crate::daemon::adapters::discord;
use crate::daemon::hover_poll_enabled;
use crate::daemon::monitor_for_overlay;
use crate::daemon::overlay::geometry;
use crate::daemon::overlay::state;
use crate::daemon::overlay::state::Overlay;
use crate::daemon::overlay::view;
use crate::daemon::resolve_command;

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
    HoverCursor(Option<(i32, i32)>),
}

/// Build and run the layer-shell overlay application.
pub(crate) fn run(cfg: Config, auth: Option<OwnAppAuth>) -> ExitCode {
    // No text input in the overlay; skip the always-on clipboard worker.
    iced_layershell::disable_clipboard();

    let size = (cfg.width, 64);
    let offset = geometry::offset(&cfg);
    let anchor = geometry::anchor(&cfg);
    let layer = if cfg.show_on_fullscreen {
        Layer::Overlay
    } else {
        Layer::Top
    };
    let start_mode = match cfg.monitor.as_deref() {
        Some(name) => StartMode::TargetScreen(name.to_string()),
        None => StartMode::Active,
    };
    let rpc_auth = DiscordRpc(auth.map(Arc::new));

    let result = application(
        move || {
            let mut overlay = Overlay::new(cfg.clone());
            overlay.hydrate_roster();
            (overlay, Task::none())
        },
        "hyprlay",
        update,
        view::view::<Message>,
    )
    .style(|_state, _theme| iced::theme::Style {
        background_color: Color::TRANSPARENT,
        text_color: Color::WHITE,
    })
    .subscription(move |state| subscription(state, &rpc_auth))
    .settings(Settings {
        layer_settings: LayerShellSettings {
            anchor,
            layer,
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
    .run();

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(event = "daemon_shell_error", error = %e, "layer-shell run failed");
            ExitCode::FAILURE
        }
    }
}

fn subscription(state: &Overlay, auth: &DiscordRpc) -> Subscription<Message> {
    let hover = if hover_poll_enabled(state) {
        Subscription::run(hover_subscription)
    } else {
        Subscription::none()
    };
    Subscription::batch([
        Subscription::run_with(auth.clone(), discord_subscription),
        Subscription::run(ctl_subscription),
        hover,
    ])
}

fn discord_subscription(rpc: &DiscordRpc) -> impl futures_util::Stream<Item = Message> + use<> {
    // `use<>`: the stream owns a backend clone, so the opaque type must stay
    // 'static instead of capturing the `&rpc` borrow (edition 2024).
    use futures_util::StreamExt;
    let backend = rpc.0.clone();
    iced::stream::channel(64, move |sender| async move {
        discord::run(sender, backend).await;
    })
    .map(Message::Discord)
}

fn ctl_subscription() -> impl futures_util::Stream<Item = Message> {
    use futures_util::StreamExt;
    crate::daemon::ctl_server::incoming().map(|req| Message::Ctl {
        command: req.command,
        reply: req.reply,
    })
}

fn hover_subscription() -> impl futures_util::Stream<Item = Message> {
    use futures_util::SinkExt;
    iced::stream::channel(
        64,
        |mut sender: iced::futures::channel::mpsc::Sender<Message>| async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(50));
            loop {
                interval.tick().await;
                let pos = tokio::task::spawn_blocking(crate::platform::cursor::cursor_pos)
                    .await
                    .unwrap_or(None);
                let _ = sender.send(Message::HoverCursor(pos)).await;
            }
        },
    )
}

fn update(state: &mut Overlay, message: Message) -> Task<Message> {
    match message {
        Message::Discord(ev) => {
            let change = state.apply_discord(ev);
            if !hover_poll_enabled(state) && state.is_hovered() {
                state.set_hovered(false);
            }
            match change {
                state::RosterChange::Changed => {
                    Task::batch([resize_task(state), avatar_tasks(state)])
                }
                state::RosterChange::Unchanged => Task::none(),
            }
        }
        Message::AvatarResult { user_id, data } => {
            if let Some(data) = data {
                state.insert_avatar(user_id, data);
            }
            Task::none()
        }
        Message::Ctl { command, reply } => handle_ctl(state, command, reply),
        Message::HoverCursor(pos) => {
            if !state.config().dim_on_hover
                || !state.config().visible
                || state.displayed().is_empty()
                || state.status() != hyprlay_core::domain::ConnectionStatus::Connected
            {
                if state.is_hovered() {
                    state.set_hovered(false);
                }
                return Task::none();
            }
            let Some((x, y)) = pos else {
                if state.is_hovered() {
                    state.set_hovered(false);
                }
                return Task::none();
            };
            let monitor = monitor_for_overlay(state.config());
            let rect = geometry::overlay_rect(
                state.config(),
                state.size(),
                state.offset(),
                monitor.as_ref(),
            );
            let now_hovered = rect.contains((x, y));
            tracing::debug!(
                event = "hover_tick",
                pos = ?pos,
                rect = ?rect,
                hovered = now_hovered,
                dim_on_hover = state.config().dim_on_hover
            );
            if now_hovered != state.is_hovered() {
                state.set_hovered(now_hovered);
            }
            Task::none()
        }
        // Variants generated by #[to_layer_message] are sent as Tasks and
        // consumed by the layershell runtime, never re-delivered here.
        _ => Task::none(),
    }
}

/// Control-socket entry point: parse once, resolve against daemon state via
/// the shared [`resolve_command`], then translate the reply, effects, and
/// lifecycle into layer-shell Tasks.
fn handle_ctl(
    state: &mut Overlay,
    command: String,
    reply: oneshot::Sender<String>,
) -> Task<Message> {
    let cmd: hyprlay_core::domain::Command = match command.parse() {
        Ok(cmd) => cmd,
        Err(err) => {
            let _ = reply.send(err);
            return Task::none();
        }
    };

    let outcome = resolve_command(state, cmd);

    let _ = reply.send(outcome.reply);

    let mut tasks: Vec<Task<Message>> = Vec::new();

    for effect in outcome.effects {
        match effect {
            hyprlay_core::domain::Effect::Resize => tasks.push(resize_task(state)),
            hyprlay_core::domain::Effect::Reanchor => {
                state.reanchor();
                let anchor = geometry::anchor(state.config());
                tasks.push(Task::done(Message::AnchorChange(anchor)));
                tasks.push(Task::done(Message::MarginChange(state.offset())));
            }
            hyprlay_core::domain::Effect::Nudge(dx, dy) => {
                state.nudge(dx, dy);
                tasks.push(Task::done(Message::MarginChange(state.offset())));
            }
        }
    }

    match outcome.lifecycle {
        Some(Lifecycle::Restart) => tasks.push(restart_daemon()),
        Some(Lifecycle::Quit) => tasks.push(clean_shutdown()),
        None => {}
    }

    Task::batch(tasks)
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
fn restart_daemon() -> Task<Message> {
    use std::os::unix::process::CommandExt;
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

/// Clean shutdown for `quit`: the exit task makes the runtime stop its event
/// loop, which drops the layer surface and ends `main` normally with exit
/// code 0. The ctl listener lives inside a subscription stream and is torn
/// down with the loop. This is the one piece of the quit path without an
/// in-process pin — it is a single runtime call, verified by live smoke.
fn clean_shutdown() -> Task<Message> {
    iced::exit()
}
