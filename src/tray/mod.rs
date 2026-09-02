//! System tray front: a resident `hyprlay-tray` process that registers a
//! system tray so the user can see daemon state and drive hyprlay without a
//! terminal.
//!
//! Design (see the tray-menu spec):
//! - One process, one thing: the tray outlives the daemon, which is what
//!   makes a real Start/Stop toggle possible.
//! - A 2 s poll loop sends `status` over the ctl socket and **diff-gates**
//!   every backend update on real state changes, so steady state emits zero
//!   traffic.
//! - Menu activate closures are non-blocking: they push a [`MenuAction`] onto
//!   an mpsc channel; the poll loop performs the side effect (ctl call /
//!   sibling spawn) and forces one immediate refresh.
//! - Single-instance guard via `hyprlay-core::singleton` (flock).
//!
//! Everything here is backend-agnostic: the menu model, the poll loop, and
//! the action channel are shared across OSes. The platform backend lives in
//! `crate::platform::tray`, selected by [`run`]'s `#[cfg]` so the Linux
//! `ksni` SNI/D-Bus item and the Windows/macOS `tray-icon` item each register
//! through the same [`Tray`] port.

pub(crate) mod daemon;
pub(crate) mod icon;
pub(crate) mod menu;
pub(crate) mod port;

use std::time::Duration;

use daemon::spawn_sibling_gui;
use hyprlay_core::daemon_control::StopPolicy;
use hyprlay_core::daemon_control::Toggle;
use hyprlay_core::domain::Command;
use hyprlay_core::domain::Key;
use hyprlay_core::domain::Value;
use hyprlay_core::singleton::AcquireError;
use menu::MenuAction;
use menu::TrayState;
use menu::parse_status;
use port::Tray;

use crate::platform::service::SystemControl;

/// The singleton lock name under `$XDG_RUNTIME_DIR`.
const LOCK_NAME: &str = "hyprlay-tray";
/// Poll cadence, matching the GUI refresh interval.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Entry point for the `hyprlay-tray` binary. Returns the process exit code.
pub fn run() -> i32 {
    // Single-instance guard held for the whole process: bind the guard to an
    // outer variable so it is not dropped at the end of the match arm. The
    // lock self-releases when this process exits.
    let _lock = match hyprlay_core::singleton::acquire(LOCK_NAME) {
        Ok(lock) => lock,
        Err(AcquireError::AlreadyHeld) => {
            eprintln!("hyprlay-tray: another instance is already running");
            return 1;
        }
        Err(e) => {
            eprintln!("hyprlay-tray: {e}");
            return 1;
        }
    };

    let connected = icon::load_icon(include_bytes!("../../assets/tray-connected.png"));
    let disconnected = icon::load_icon(include_bytes!("../../assets/tray-disconnected.png"));

    #[cfg(target_os = "linux")]
    {
        crate::platform::tray::ksni::run(connected, disconnected)
    }
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        crate::platform::tray::tray_icon::run(connected, disconnected)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        tracing::error!(
            event = "tray_unsupported_target",
            "this target has no system-tray backend"
        );
        1
    }
}

/// The shared poll loop: on a 2 s cadence it diff-gates the live daemon state
/// against the last rendered snapshot and pushes real changes to the backend;
/// on a channel `MenuAction` it performs the side effect then forces one
/// immediate refresh. A `Quit` action or a closed channel tears the backend
/// down and leaves the loop.
pub(crate) async fn poll_loop<T: Tray>(
    tray: &mut T,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<MenuAction>,
) -> i32 {
    let mut last: Option<TrayState> = None;
    // Render the initial state so the icon/menu appear immediately.
    refresh(tray, &mut last).await;

    let mut interval = tokio::time::interval(POLL_INTERVAL);
    loop {
        tokio::select! {
            _ = interval.tick() => {
                refresh(tray, &mut last).await;
            }
            action = rx.recv() => {
                match action {
                    // Channel closed or Quit: leave the loop. The backend's
                    // `shutdown` unregisters; dropping it afterwards is
                    // harmless.
                    None | Some(MenuAction::Quit) => {
                        tray.shutdown().await;
                        break;
                    }
                    Some(action) => {
                        perform(action, last.as_ref()).await;
                        // Force one immediate refresh after the side effect.
                        refresh(tray, &mut last).await;
                    }
                }
            }
        }
    }
    0
}

/// Poll the daemon and push the new state to the backend only when it differs
/// from the last rendered snapshot (diff-gate: steady state is silent).
async fn refresh<T: Tray>(tray: &mut T, last: &mut Option<TrayState>) {
    let new = poll_state().await;
    if last.as_ref() != Some(&new) {
        tray.update(&new).await;
        *last = Some(new);
    }
}

/// Blocking `status` round-trip off the async loop, mirroring the GUI's
/// `send` helper (`gui/mod.rs:236`).
async fn poll_state() -> TrayState {
    let reply = tokio::task::spawn_blocking(|| {
        hyprlay_core::ctl::send_command_line(&crate::platform::ipc::control::Control, "status")
    })
    .await
    .unwrap_or(None);
    match reply {
        Some(reply) => parse_status(&reply),
        None => TrayState::down(),
    }
}

/// Perform one user action, then the loop forces a refresh.
async fn perform(action: MenuAction, state: Option<&TrayState>) {
    match action {
        MenuAction::OpenSettings => match tokio::task::spawn_blocking(spawn_sibling_gui).await {
            Ok(Ok(())) => {
                tracing::info!(
                    event = "tray_open_settings",
                    "spawned or focused hyprlay-gui"
                );
            }
            Ok(Err(e)) => {
                tracing::warn!(
                    event = "tray_open_settings_failed",
                    error = %e,
                    "could not open settings"
                );
            }
            Err(e) => {
                tracing::error!(
                    event = "tray_open_settings_join_failed",
                    error = %e,
                    "spawn_blocking for hyprlay-gui failed"
                );
            }
        },
        MenuAction::ToggleVisible => {
            let visible = state.map(|s| s.visible).unwrap_or(false);
            let command = Command::Set(Key::Visible, Value::Flag(!visible));
            tokio::task::spawn_blocking(move || {
                hyprlay_core::ctl::send_command_line(
                    &crate::platform::ipc::control::Control,
                    &command.to_string(),
                )
            })
            .await
            .ok();
        }
        MenuAction::ToggleDaemon => {
            let up = state.map(|s| s.up).unwrap_or(false);
            let toggle = if up { Toggle::Stop } else { Toggle::Start };
            let _ = tokio::task::spawn_blocking(move || {
                hyprlay_core::daemon_control::execute_toggle(
                    &SystemControl,
                    toggle,
                    StopPolicy::ViaSocket,
                )
            })
            .await;
        }
        MenuAction::Quit => unreachable!("Quit is handled by the loop before perform"),
    }
}
