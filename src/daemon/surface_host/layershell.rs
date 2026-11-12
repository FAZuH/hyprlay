//! Linux/Wayland surface host: the existing `iced_layershell` shell. Carved
//! out of `daemon/mod.rs` into its own arm, and unchanged apart from the
//! surface height: same anchor vocabulary, layer, `StartMode`,
//! keyboard-interactivity and transparent-events settings. This module only
//! runs on Linux, where iced_layershell (and iced's `wayland` feature) are
//! available.

use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::Mutex;

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
use crate::daemon::monitor_for_overlay;
use crate::daemon::overlay::geometry;
use crate::daemon::overlay::state;
use crate::daemon::overlay::state::Overlay;
use crate::daemon::overlay::view;
use crate::daemon::resolve_command;
use crate::daemon::surface_host::boot_size;
use crate::daemon::surface_host::take_boot;

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
/// The one conversion from [`geometry::SurfaceAnchor`] to the layer-shell
/// `Anchor`. Lives here because this is the sanctioned composition point for
/// platform crates (ADR-004): `geometry` stays free of `iced_layershell`
/// imports and this module owns the renderer's types.
fn to_wayland_anchor(anchor: geometry::SurfaceAnchor) -> iced_layershell::reexport::Anchor {
    use iced_layershell::reexport::Anchor;
    // The same construction the original inline code used: bitflag `|` over
    // the four edges, which is what the wlr layer-shell protocol defines.
    let mut out = Anchor::empty();
    if anchor.0 {
        out |= Anchor::Top;
    }
    if anchor.1 {
        out |= Anchor::Right;
    }
    if anchor.2 {
        out |= Anchor::Bottom;
    }
    if anchor.3 {
        out |= Anchor::Left;
    }
    out
}

/// Resolve the Wayland socket from the two variables the protocol defines.
/// Pure so the mapping is testable without touching the process environment:
/// an absolute `WAYLAND_DISPLAY` is the socket itself, a bare one is a name
/// under `XDG_RUNTIME_DIR`.
fn socket_path(
    wayland_display: Option<&OsStr>,
    xdg_runtime_dir: Option<&OsStr>,
) -> Result<PathBuf, String> {
    let display = wayland_display.ok_or_else(|| "WAYLAND_DISPLAY is not set".to_string())?;
    let display = Path::new(display);
    if display.is_absolute() {
        return Ok(display.to_path_buf());
    }
    let runtime = xdg_runtime_dir.ok_or_else(|| "XDG_RUNTIME_DIR is not set".to_string())?;
    Ok(Path::new(runtime).join(display))
}

/// Connect once to prove a compositor is listening. A stale socket file left
/// by a dead compositor fails here rather than inside `iced_layershell`.
fn ensure_compositor() -> Result<(), String> {
    let path = socket_path(
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
        std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
    )?;
    std::os::unix::net::UnixStream::connect(&path)
        .map(|_| ())
        .map_err(|error| format!("cannot connect to {}: {error}", path.display()))
}

pub(crate) fn run(cfg: Config, auth: Option<OwnAppAuth>) -> ExitCode {
    // `iced_layershell` panics on a missing compositor
    // (`ConnectError(NoCompositor)`), which aborts the process and dumps core.
    // A plain error exit lets systemd's restart policy retry instead.
    if let Err(reason) = ensure_compositor() {
        tracing::error!(
            event = "daemon_no_compositor",
            reason = %reason,
            "no Wayland compositor to attach the overlay to; exiting"
        );
        return ExitCode::FAILURE;
    }

    // No text input in the overlay; skip the always-on clipboard worker.
    iced_layershell::disable_clipboard();

    // One overlay for both the surface size and the boot closure, so the
    // height the surface is created at and the height the model reports are
    // never two different reads of the roster cache.
    let overlay = Overlay::boot(cfg.clone());
    let size = boot_size(&overlay);
    let boot_overlay = Mutex::new(Some(overlay));
    let offset = geometry::offset(&cfg);
    // The one conversion from geometry's own anchor vocabulary to the
    // renderer's type. This is the sanctioned composition point for platform
    // crates (ADR-004), so the conversion lives here and geometry stays free
    // of iced_layershell imports.
    let anchor = to_wayland_anchor(geometry::anchor(&cfg));
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
        move || (take_boot(&boot_overlay, &cfg), Task::none()),
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
    let hover = if state.hover_polling() {
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
            if !state.hover_polling() {
                state.clear_hover_if_set();
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
            // Same guard as the Discord arm above, which calls the shared
            // helper; the winit arm does too. The audit found this arm
            // re-implemented it inline with a De Morgan'd `||` and had
            // already diverged, so it is the shared helper now.
            if !state.hover_polling() {
                state.clear_hover_if_set();
                return Task::none();
            }
            let Some((x, y)) = pos else {
                state.clear_hover_if_set();
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

    let _ = reply.send(outcome.reply.text().to_string());

    let mut tasks: Vec<Task<Message>> = Vec::new();

    for effect in outcome.effects {
        match effect {
            hyprlay_core::domain::Effect::Resize => tasks.push(resize_task(state)),
            hyprlay_core::domain::Effect::Reanchor => {
                state.reanchor();
                // Same conversion as the boot path above: geometry owns the
                // vocabulary, this module owns the renderer's type.
                let anchor = to_wayland_anchor(geometry::anchor(state.config()));
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

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::PathBuf;

    use super::socket_path;

    #[test]
    fn an_absolute_wayland_display_is_the_socket_itself() {
        let path = socket_path(Some(OsStr::new("/run/user/1000/wayland-9")), None)
            .expect("absolute display resolves without a runtime dir");

        assert_eq!(path, PathBuf::from("/run/user/1000/wayland-9"));
    }

    #[test]
    fn a_bare_wayland_display_lives_under_the_runtime_dir() {
        let path = socket_path(
            Some(OsStr::new("wayland-1")),
            Some(OsStr::new("/run/user/1000")),
        )
        .expect("a bare display resolves under the runtime dir");

        assert_eq!(path, PathBuf::from("/run/user/1000/wayland-1"));
    }

    #[test]
    fn no_wayland_display_is_an_error() {
        assert!(socket_path(None, Some(OsStr::new("/run/user/1000"))).is_err());
    }

    #[test]
    fn a_bare_wayland_display_without_a_runtime_dir_is_an_error() {
        assert!(socket_path(Some(OsStr::new("wayland-1")), None).is_err());
    }
}
