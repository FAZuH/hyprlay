//! Windows/macOS tray backend via `tray-icon` (`Shell_NotifyIcon` /
//! `NSStatusItem`).
//!
//! `tray-icon` must be created and its menu events processed on the event-loop
//! thread — on macOS that is the main thread. The front is its own process, so
//! this adapter runs the OS event loop on the main thread and drives the shared
//! poll loop on a background runtime thread, bridging the two over channels:
//! menu activations flow main → poll loop, state snapshots flow poll loop →
//! main (which owns the `TrayIcon`), and the quit signal flows poll loop →
//! main.
//!
//! The [`Tray`](crate::tray::Tray) port absorbs this event-loop difference:
//! on Linux `ksni` registers a D-Bus item directly on the current-thread
//! runtime; here the port is a thin channel marshaler.

use std::collections::HashMap;
use std::time::Duration;

use tray_icon::Icon;
use tray_icon::TrayIcon;
use tray_icon::TrayIconBuilder;
use tray_icon::menu::Menu;
use tray_icon::menu::MenuEvent;
use tray_icon::menu::MenuId;
use tray_icon::menu::MenuItem;
use tray_icon::menu::PredefinedMenuItem;

use crate::tray::icon::IconData;
use crate::tray::menu::MenuAction;
use crate::tray::menu::MenuRow;
use crate::tray::menu::TrayState;
use crate::tray::menu::build_menu;
use crate::tray::port::Tray;

/// The [`Tray`] port behind the `tray-icon` backend: a channel marshaler
/// between the shared poll loop (background thread) and the main-thread event
/// loop that owns the `TrayIcon`.
pub struct TrayIconPort {
    update_tx: tokio::sync::mpsc::UnboundedSender<TrayState>,
    quit_tx: tokio::sync::mpsc::UnboundedSender<()>,
}

impl Tray for TrayIconPort {
    async fn update(&mut self, state: &TrayState) {
        let tx = self.update_tx.clone();
        let state = state.clone();
        // The main-thread loop owns the `TrayIcon`; it applies the snapshot.
        let _ = tx.send(state);
    }

    async fn shutdown(&mut self) {
        let tx = self.quit_tx.clone();
        // Ask the main-thread loop to tear the tray down and exit.
        let _ = tx.send(());
    }
}

/// Create the tray on the main/event-loop thread and drive the rest of the
/// process from it. Returns the process exit code.
pub fn run(connected: IconData, disconnected: IconData) -> i32 {
    let (action_tx, mut action_rx) = tokio::sync::mpsc::unbounded_channel::<MenuAction>();
    let (update_tx, mut update_rx) = tokio::sync::mpsc::unbounded_channel::<TrayState>();
    let (quit_tx, mut quit_rx) = tokio::sync::mpsc::unbounded_channel::<()>();

    // The tray icon must be created on the thread that runs the OS event loop.
    let (mut tray, mut actions) = match create_tray(&disconnected) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(
                event = "tray_create_failed",
                error = %e,
                "could not create the system tray icon"
            );
            return 1;
        }
    };

    // Shared poll loop on a background runtime thread. It pushes state
    // snapshots to `update_tx` and the quit signal to `quit_tx`; the main loop
    // below owns the `TrayIcon` and applies them.
    let poll = std::thread::spawn(move || {
        let mut port = TrayIconPort { update_tx, quit_tx };
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("hyprlay tray: could not start runtime");
        rt.block_on(crate::tray::poll_loop(&mut port, &mut action_rx))
    });

    let code = main_loop(
        &mut tray,
        &mut actions,
        &connected,
        &disconnected,
        &action_tx,
        &mut update_rx,
        &mut quit_rx,
    );
    let _ = poll.join();
    code
}

fn create_tray(disconnected: &IconData) -> Result<(TrayIcon, HashMap<MenuId, MenuAction>), String> {
    let (menu, actions) = build_platform_menu(&TrayState::down());
    let tray = TrayIconBuilder::new()
        .with_icon(to_tray_icon(disconnected))
        .with_tooltip("hyprlay")
        .with_menu(Box::new(menu))
        .build()
        .map_err(|e| e.to_string())?;
    Ok((tray, actions))
}

/// The main-thread event loop. A real OS event pump (winit / NSApplication /
/// win32 `GetMessage`) slots in here to keep menu events flowing; the polling
/// skeleton below covers the channel plumbing the port needs.
fn main_loop(
    tray: &mut TrayIcon,
    actions: &mut HashMap<MenuId, MenuAction>,
    connected: &IconData,
    disconnected: &IconData,
    action_tx: &tokio::sync::mpsc::UnboundedSender<MenuAction>,
    update_rx: &mut tokio::sync::mpsc::UnboundedReceiver<TrayState>,
    quit_rx: &mut tokio::sync::mpsc::UnboundedReceiver<()>,
) -> i32 {
    loop {
        // Forward menu activations to the poll loop.
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if let Some(action) = actions.get(&event.id) {
                let _ = action_tx.send(*action);
            }
        }
        // Apply state snapshots that arrived from the poll loop.
        while let Ok(state) = update_rx.try_recv() {
            *actions = apply_state(tray, connected, disconnected, &state);
        }
        // Quit: the poll loop's `shutdown` signals the main loop.
        if quit_rx.try_recv().is_ok() {
            return 0;
        }
        // Yield so the OS can deliver its events without busy-spinning.
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn apply_state(
    tray: &TrayIcon,
    connected: &IconData,
    disconnected: &IconData,
    state: &TrayState,
) -> HashMap<MenuId, MenuAction> {
    let (menu, actions) = build_platform_menu(state);
    // `set_menu` returns `()`; the icon/tooltip setters return `Result`.
    tray.set_menu(Some(Box::new(menu)));
    let icon = if state.up { connected } else { disconnected };
    let _ = tray.set_icon(Some(to_tray_icon(icon)));
    let _ = tray.set_tooltip(Some("hyprlay"));
    actions
}

/// Build a `tray_icon` menu (and the id → [`MenuAction`] map it carries) from
/// the shared menu model. Shared with the Linux adapter via [`build_menu`].
fn build_platform_menu(state: &TrayState) -> (Menu, HashMap<MenuId, MenuAction>) {
    let menu = Menu::new();
    let mut actions = HashMap::new();
    for row in build_menu(state) {
        let result = match row {
            MenuRow::Status(label) => menu.append(&MenuItem::new(label, false, None)),
            MenuRow::Separator => menu.append(&PredefinedMenuItem::separator()),
            MenuRow::Item { label, action } => {
                let item = MenuItem::new(label, true, None);
                actions.insert(item.id().clone(), action);
                menu.append(&item)
            }
        };
        if let Err(e) = result {
            tracing::warn!(
                event = "tray_menu_append_failed",
                error = %e,
                "could not add a menu row"
            );
        }
    }
    (menu, actions)
}

fn to_tray_icon(data: &IconData) -> Icon {
    Icon::from_rgba(data.rgba.clone(), data.width, data.height)
        .expect("tray icon RGBA must be valid")
}
