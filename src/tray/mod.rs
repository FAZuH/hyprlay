//! System tray front: a resident `hyprlay-tray` process that registers a
//! StatusNotifierItem (ksni over DBus) so the user can see daemon state and
//! drive hyprlay without a terminal.
//!
//! Design (see the tray-menu spec):
//! - One process, one thing: the tray outlives the daemon, which is what
//!   makes a real Start/Stop toggle possible.
//! - A 2 s poll loop sends `status` over the ctl socket and **diff-gates**
//!   every `handle.update` on real state changes, so steady state emits zero
//!   DBus traffic.
//! - Menu activate closures are non-blocking: they push a [`MenuAction`] onto
//!   an mpsc channel; the poll loop performs the side effect (ctl call /
//!   sibling spawn) and forces one immediate refresh.
//! - Single-instance guard via `hyprlay-core::singleton` (flock).

mod daemon;
mod icon;
mod menu;

use std::time::Duration;

use daemon::spawn_sibling_gui;
use hyprlay_core::daemon_control::StopPolicy;
use hyprlay_core::daemon_control::SystemControl;
use hyprlay_core::daemon_control::Toggle;
use hyprlay_core::domain::Command;
use hyprlay_core::domain::Key;
use hyprlay_core::domain::Value;
use hyprlay_core::singleton::AcquireError;
use ksni::TrayMethods;
use menu::MenuAction;
use menu::MenuRow;
use menu::TrayState;
use menu::build_menu;
use menu::parse_status;

/// The singleton lock name under `$XDG_RUNTIME_DIR`.
const LOCK_NAME: &str = "hyprlay-tray";
/// Poll cadence, matching the GUI refresh interval.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// The ksni tray implementation. Holds the live [`TrayState`] (read by
/// `menu`/`icon_pixmap`) and the action channel sender (written by menu
/// activate closures).
pub struct Tray {
    state: TrayState,
    tx: tokio::sync::mpsc::UnboundedSender<MenuAction>,
    connected_icon: ksni::Icon,
    disconnected_icon: ksni::Icon,
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "hyprlay-tray".into()
    }

    fn title(&self) -> String {
        "hyprlay".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![if self.state.up {
            self.connected_icon.clone()
        } else {
            self.disconnected_icon.clone()
        }]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            icon_name: String::new(),
            icon_pixmap: self.icon_pixmap(),
            title: "hyprlay".into(),
            description: self.state.summary.clone(),
        }
    }

    fn menu(&self) -> Vec<ksni::menu::MenuItem<Self>> {
        build_menu(&self.state)
            .into_iter()
            .map(|row| match row {
                MenuRow::Status(label) => ksni::menu::StandardItem {
                    label,
                    enabled: false,
                    ..Default::default()
                }
                .into(),
                MenuRow::Separator => ksni::menu::MenuItem::Separator,
                MenuRow::Item { label, action } => ksni::menu::StandardItem {
                    label,
                    enabled: true,
                    // Non-blocking: hand the action to the poll loop.
                    activate: Box::new(move |tray: &mut Tray| {
                        let _ = tray.tx.send(action);
                    }),
                    ..Default::default()
                }
                .into(),
            })
            .collect()
    }

    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        // No StatusNotifierWatcher host (e.g. waybar not started yet): log
        // once and keep running idle — never fatal. A late-starting host will
        // bring the icon up on its own.
        static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !WARNED.swap(true, std::sync::atomic::Ordering::SeqCst) {
            tracing::warn!(
                event = "tray_watcher_offline",
                "no StatusNotifierWatcher host; tray stays idle until one appears"
            );
        }
        true
    }
}

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

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("hyprlay-tray: could not start runtime");
    rt.block_on(run_async())
}

async fn run_async() -> i32 {
    let connected = icon::load_icon(include_bytes!("../../assets/tray-connected.png"));
    let disconnected = icon::load_icon(include_bytes!("../../assets/tray-disconnected.png"));

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<MenuAction>();
    let tray = Tray {
        state: TrayState::down(),
        tx,
        connected_icon: connected,
        disconnected_icon: disconnected,
    };

    // `assume_sni_available` routes a missing Watcher/WontShow to
    // `watcher_offline` instead of failing spawn, so a late host still
    // works. A hard D-Bus error remains fatal.
    let handle = match tray.assume_sni_available(true).spawn().await {
        Ok(handle) => handle,
        Err(e) => {
            tracing::error!(
                event = "tray_spawn_failed",
                error = %e,
                "could not register the system tray"
            );
            return 1;
        }
    };

    let mut last: Option<TrayState> = None;
    // Render the initial state so the icon/menu appear immediately.
    refresh(&handle, &mut last).await;

    let mut interval = tokio::time::interval(POLL_INTERVAL);
    loop {
        tokio::select! {
            _ = interval.tick() => {
                refresh(&handle, &mut last).await;
            }
            action = rx.recv() => {
                match action {
                    // Channel closed or Quit: leave the loop; dropping
                    // `handle` unregisters the item.
                    None | Some(MenuAction::Quit) => break,
                    Some(action) => {
                        perform(action, last.as_ref()).await;
                        // Force one immediate refresh after the side effect.
                        refresh(&handle, &mut last).await;
                    }
                }
            }
        }
    }
    0
}

/// Poll the daemon and push the new state to the tray only when it differs
/// from the last rendered snapshot (diff-gate: steady state is silent).
async fn refresh(handle: &ksni::Handle<Tray>, last: &mut Option<TrayState>) {
    let new = poll_state().await;
    if last.as_ref() != Some(&new) {
        handle.update(|tray| tray.state = new.clone()).await;
        *last = Some(new);
    }
}

/// Blocking `status` round-trip off the async loop, mirroring the GUI's
/// `send` helper (`gui/mod.rs:236`).
async fn poll_state() -> TrayState {
    let reply = tokio::task::spawn_blocking(|| hyprlay_core::ctl::send_command_line("status"))
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
                hyprlay_core::ctl::send_command_line(&command.to_string())
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
