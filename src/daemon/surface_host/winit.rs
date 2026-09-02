//! Windows/macOS surface host: a frameless, transparent, always-on-top
//! `iced` window moved to the computed on-screen position. This arm
//! compiles only off-Linux — the layer-shell arm serves every Linux
//! target, so X11 sessions get no overlay host; it shares
//! the roster view, the pure state machine, and the geometry math with the
//! layer-shell arm, and only differs in how the surface is created, placed,
//! resized, and how hover is detected.

use std::process::ExitCode;
use std::sync::Arc;

use hyprlay_core::config::Config;
use hyprlay_core::platform::Platform;
use iced::Color;
use iced::Element;
use iced::Subscription;
use iced::Task;
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

/// The state of the winit overlay. Wraps the shared [`Overlay`] model and
/// adds the winit-only plumbing: the window id (needed for `resize`/`move_to`
/// tasks, learned asynchronously after boot) and the last frame we placed, so
/// hover compares the global cursor against the *actual* on-screen rect
/// instead of the layer-shell computed rect.
struct WinitState {
    overlay: Overlay,
    window: Option<iced::window::Id>,
    last_frame: Option<geometry::WinitFrame>,
}

/// App message for the winit arm: the shared domain messages plus one that
/// carries the window id resolved after boot. No `#[to_layer_message]`
/// geometry variants — placement is applied via `iced::window` tasks instead.
#[derive(Debug)]
enum Message {
    WindowId(Option<iced::window::Id>),
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

/// Build and run the winit overlay application.
pub(crate) fn run(cfg: Config, auth: Option<OwnAppAuth>) -> ExitCode {
    let start_size = (cfg.width, 64);
    let offset = geometry::offset(&cfg);
    let monitor = monitor_for_overlay(&cfg);
    // The winit arm places the window absolutely at the anchored logical
    // frame, so the initial Settings and the boot-state frame agree.
    let frame = geometry::winit_frame(&cfg, start_size, offset, monitor.as_ref());
    let position = iced::window::Position::Specific(iced::Point::new(frame.x, frame.y));

    let rpc_auth = DiscordRpc(auth.map(Arc::new));
    let cfg_for_boot = cfg.clone();

    // `iced::application` takes (boot, update, view) — unlike the layer-shell
    // builder there is no namespace argument. The window title defaults to
    // the app's name from the state type, which is cosmetic for an always-on-
    // top overlay.
    let result = iced::application(
        move || {
            let mut overlay = Overlay::new(cfg_for_boot.clone());
            overlay.hydrate_roster();
            let window = iced::window::latest().map(Message::WindowId);
            (
                WinitState {
                    overlay,
                    window: None,
                    last_frame: Some(frame),
                },
                window,
            )
        },
        update,
        view_window,
    )
    .window(iced::window::Settings {
        size: iced::Size::new(frame.w, frame.h.max(0.0)),
        position,
        decorations: false,
        transparent: true,
        resizable: false,
        visible: true,
        icon: Some(crate::platform::icon::window_icon()),
        level: iced::window::Level::AlwaysOnTop,
        ..Default::default()
    })
    .style(|_state, _theme| iced::theme::Style {
        background_color: Color::TRANSPARENT,
        text_color: Color::WHITE,
    })
    .subscription(move |state| subscription(state, &rpc_auth))
    .run();

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(event = "daemon_shell_error", error = %e, "winit run failed");
            ExitCode::FAILURE
        }
    }
}

/// The winit arm's view: delegates to the shared generic [`view::view`]. A
/// named fn-item rather than a closure so iced's higher-ranked `ViewFn` bound
/// is satisfied (a closure reborrowing through `&state.overlay` cannot prove
/// the "input borrow outlives the returned element" relationship).
fn view_window<'a>(state: &'a WinitState) -> Element<'a, Message> {
    view::view::<Message>(&state.overlay)
}

fn subscription(state: &WinitState, auth: &DiscordRpc) -> Subscription<Message> {
    let hover = if hover_poll_enabled(&state.overlay) {
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

fn update(state: &mut WinitState, message: Message) -> Task<Message> {
    match message {
        Message::WindowId(id) => {
            if let Some(id) = id {
                state.window = Some(id);
                // Click-through: the overlay is pure display, every pointer
                // event passes through to whatever is below.
                iced::window::enable_mouse_passthrough(id)
            } else {
                Task::none()
            }
        }
        Message::Discord(ev) => {
            let change = state.overlay.apply_discord(ev);
            if !hover_poll_enabled(&state.overlay) && state.overlay.is_hovered() {
                state.overlay.set_hovered(false);
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
                state.overlay.insert_avatar(user_id, data);
            }
            Task::none()
        }
        Message::Ctl { command, reply } => handle_ctl(state, command, reply),
        Message::HoverCursor(pos) => {
            if !hover_poll_enabled(&state.overlay) {
                if state.overlay.is_hovered() {
                    state.overlay.set_hovered(false);
                }
                return Task::none();
            }
            let Some((x, y)) = pos else {
                if state.overlay.is_hovered() {
                    state.overlay.set_hovered(false);
                }
                return Task::none();
            };
            let rect = hover_rect(state);
            let now_hovered = rect.contains((x, y));
            tracing::debug!(
                event = "hover_tick",
                pos = ?pos,
                rect = ?rect,
                hovered = now_hovered,
                dim_on_hover = state.overlay.config().dim_on_hover
            );
            if now_hovered != state.overlay.is_hovered() {
                state.overlay.set_hovered(now_hovered);
            }
            Task::none()
        }
    }
}

/// Control-socket entry point: parse once, resolve against daemon state via
/// the shared [`resolve_command`], then translate the reply, effects, and
/// lifecycle into winit window Tasks.
fn handle_ctl(
    state: &mut WinitState,
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

    let outcome = resolve_command(&mut state.overlay, cmd);

    let _ = reply.send(outcome.reply);

    let mut tasks: Vec<Task<Message>> = Vec::new();

    for effect in outcome.effects {
        match effect {
            hyprlay_core::domain::Effect::Resize => tasks.push(resize_task(state)),
            hyprlay_core::domain::Effect::Reanchor => {
                state.overlay.reanchor();
                tasks.push(move_window(state));
            }
            hyprlay_core::domain::Effect::Nudge(dx, dy) => {
                state.overlay.nudge(dx, dy);
                tasks.push(move_window(state));
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

/// The rect the winit arm hovers against: the last frame we placed, falling
/// back to the freshly-computed anchored frame before the window id is known.
fn hover_rect(state: &WinitState) -> geometry::Rect {
    let frame = state.last_frame.unwrap_or_else(|| {
        let cfg = state.overlay.config().clone();
        let monitor = monitor_for_overlay(&cfg);
        geometry::winit_frame(
            &cfg,
            state.overlay.size(),
            state.overlay.offset(),
            monitor.as_ref(),
        )
    });
    geometry::Rect {
        x: frame.x as i32,
        y: frame.y as i32,
        w: frame.w as u32,
        h: frame.h as u32,
    }
}

/// Match the window size to the current roster; emits a resize only when the
/// size actually changed and the window id is known.
fn resize_task(state: &mut WinitState) -> Task<Message> {
    match (state.overlay.take_size_change(), state.window) {
        (Some((w, h)), Some(id)) => iced::window::resize(id, iced::Size::new(w as f32, h as f32)),
        _ => Task::none(),
    }
}

/// Recompute the anchored frame and move the winit window there, caching the
/// frame for the hover rect.
fn move_window(state: &mut WinitState) -> Task<Message> {
    let Some(id) = state.window else {
        return Task::none();
    };
    let cfg = state.overlay.config().clone();
    let size = state.overlay.size();
    let offset = state.overlay.offset();
    let monitor = monitor_for_overlay(&cfg);
    let frame = geometry::winit_frame(&cfg, size, offset, monitor.as_ref());
    state.last_frame = Some(frame);
    iced::window::move_to(id, iced::Point::new(frame.x, frame.y))
}

/// Kick off fetches for any participant avatars we don't have yet.
fn avatar_tasks(state: &mut WinitState) -> Task<Message> {
    let missing = state.overlay.claim_missing_avatars();
    Task::batch(missing.into_iter().map(|(user_id, hash, url)| {
        Task::perform(avatar::fetch(user_id.clone(), hash, url), move |data| {
            Message::AvatarResult {
                user_id: user_id.clone(),
                data,
            }
        })
    }))
}

/// Restart on the winit arm: spawn a fresh daemon process and exit this one.
/// There is no portable exec here, so the new daemon is the same binary
/// re-launched detached; `quit`/'restart' semantics match the layer-shell arm.
fn restart_daemon() -> Task<Message> {
    if let Ok(exe) = std::env::current_exe() {
        let mut cmd = std::process::Command::new(&exe);
        cmd.arg("daemon");
        // Detach the child (own process group / CREATE_NEW_PROCESS_GROUP +
        // null stdio + no wait) so the sibling outlives this process.
        match crate::platform::host::host().spawn(&mut cmd) {
            Ok(_) => return clean_shutdown(),
            Err(e) => tracing::error!(
                event = "daemon_restart_failed",
                error = %e,
                "could not spawn daemon"
            ),
        }
    }
    Task::none()
}

/// Clean shutdown for `quit`: stop the event loop so the window is torn down
/// and `main` ends normally with exit code 0.
fn clean_shutdown() -> Task<Message> {
    iced::exit()
}
