//! Settings GUI (`hyprlay gui`): a normal desktop window that drives
//! the running daemon through the same control-socket command seam the CLI
//! uses. No settings logic lives here — every change is a [`Command`], the
//! daemon stays the single source of truth.
//!
//! One deliberate exception: Discord credentials never travel the ctl
//! socket (they must not be leakable through it), so the Connection section
//! writes auth.json directly and then sends an opaque `restart` for the
//! daemon to re-detect its backend.
//!
//! Layout: header (title, search, clear-changes, reset, save) / sidebar
//! (section anchors) / one scrollable page holding every section /
//! labeled status bar (unsaved marker, Start/Stop toggle, daemon state,
//! last reply). Sidebar buttons and Ctrl+1..5 scroll the page to a
//! section's header; while the user scrolls, the sidebar highlight
//! follows the section under the top of the viewport. Typing in the
//! search box swaps the page for a flat list of matches (by label,
//! tooltip, and section name); clearing it returns to the page at the
//! offset held before the search. Every field has a tooltip. Changes
//! apply live but are NOT persisted until "Save"; closing with unsaved
//! changes is allowed (the daemon keeps runtime state, but a daemon
//! restart reverts to config.toml).

mod commands;
mod daemon;
mod fields;
mod picker;
mod scroll;
mod theme;
mod update;
mod view;

use std::collections::HashMap;
use std::sync::Arc;

use daemon::AutoStart;
use daemon::DaemonState;
use fields::Section;
use hyprlay_core::config::Config;
use hyprlay_core::config::HorizontalAnchor as H;
use hyprlay_core::config::VerticalAnchor as V;
use hyprlay_core::config::{self};
use hyprlay_core::ctl;
use hyprlay_core::daemon_control::DaemonControl;
use hyprlay_core::domain::Command;
use hyprlay_core::domain::Key;
use hyprlay_core::singleton::AcquireError;
use iced::Point;
use iced::Subscription;
use iced::Task;
use iced::keyboard::{self};
use picker::ColorTarget;
use theme::theme_for;
use update::update;
use view::view;

use crate::platform::service::SystemControl;

#[derive(Debug, Clone)]
enum Message {
    Position(H, V),
    /// Pin the vertical glue edge explicitly (or return it to Auto).
    Anchor(hyprlay_core::config::AnchorMode),
    /// Flip one boolean config key (rtl, talking-only, own user).
    SetFlag(Key, bool),
    /// Integer text edited for a numeric knob; invalid or out-of-range
    /// input is kept as a draft instead of snapping back.
    NumText(Key, String),
    /// Numeric slider moved; the value arrives inside the slider envelope.
    NumDrag(Key, f32),
    /// Restore one numeric knob to its default.
    NumReset(Key),
    ColorPart(ColorTarget, u8, f32),
    ColorHex(ColorTarget, String),
    /// Expand / collapse one color editor's HSV picker.
    PickerToggle(ColorTarget),
    /// Press / hover-move inside a picker plane; applies while dragging.
    SvPress(ColorTarget),
    SvMove(ColorTarget, Point),
    HuePress(ColorTarget),
    HueMove(ColorTarget, Point),
    PickerRelease,
    Palette(usize),
    /// Sidebar button or Ctrl+1..5 pressed: clear any search (D3), then
    /// scroll the one-page view to the section's header.
    Navigate(Section),
    /// The one-page content scrolled; the payload is the pixel offset of
    /// the top of the viewport within the content (scrollspy tracking).
    Scrolled(f32),
    /// Layout measurement of the one-page content: per-section header
    /// offsets, the scrollable range, and the navigation intent (`jump`)
    /// the measure was spawned with.
    Measured {
        offsets: [f32; Section::ALL.len()],
        max_scroll: f32,
        jump: Option<Section>,
    },
    Search(String),
    KeyPressed(keyboard::Event),
    Save,
    /// Revert the daemon's runtime config to what is on disk.
    ClearChanges,
    ResetAll,
    ResetSection(Section),
    SwitchMonitor(Option<String>),
    Monitors(Vec<String>),
    Applied(String),
    RefreshStatus,
    /// Bottom-left toggle pressed; meaning (Start/Stop) is decided from the
    /// live state at press time, never baked into the message.
    ToggleDaemon,
    /// Outcome of a toggle action: the failure text for the status line,
    /// or `None` on silent success (the follow-up refresh shows the effect).
    ToggleResult(Option<String>),
    /// Edited draft of the own-app client id; committed via AuthApply.
    AuthClientId(String),
    /// Edited draft of the own-app client secret; committed via AuthApply.
    AuthClientSecret(String),
    /// Write both credential drafts to auth.json, then restart the daemon
    /// so the new backend takes effect.
    AuthApply,
}

pub struct Gui {
    /// Local mirror of the daemon config; updated optimistically on change.
    config: Config,
    /// In-progress hex text per color editor, kept only while invalid so the
    /// text input doesn't snap back while typing; cleared on a valid commit.
    drafts: HashMap<ColorTarget, String>,
    /// Same idea as `drafts`, but for numeric inputs keyed by config key.
    num_drafts: HashMap<Key, String>,
    /// Last daemon reply (or connection error), labeled in the status bar.
    last_reply: String,
    /// Probe-driven view of the daemon: connecting → up/down.
    daemon_state: DaemonState,
    /// Boot watcher that auto-starts the daemon when the first probe
    /// proves it down; retired once settled (see `daemon::AutoStart`).
    auto_start: AutoStart,
    /// Process/socket boundary behind the Start/Stop toggle, injectable so
    /// tests can drive the same decision logic without systemctl.
    control: Arc<dyn DaemonControl>,
    /// True when the daemon's runtime config differs from config.toml.
    dirty: bool,
    monitors: Vec<String>,
    /// Active section: set immediately on navigation, then kept in sync
    /// with the section under the top of the viewport while scrolling
    /// (scrollspy). Drives the sidebar highlight and Ctrl+R's target.
    section: Section,
    search: String,
    /// Scroll offset of the one-page content, tracked from Scrolled.
    /// Frozen while the search page is up (its scrollable reports nothing)
    /// and used to restore the position on search-clear.
    last_scroll_y: f32,
    /// Which color editor has its HSV picker expanded.
    picker: Option<ColorTarget>,
    /// True while a picker plane is held down; moves then adjust the color.
    picker_drag: bool,
    /// Last cursor position seen inside a picker plane, so a press without a
    /// preceding move still picks the right color.
    picker_pos: Point,
    /// Draft of the own-app Discord client id, preloaded from auth.json.
    auth_client_id: String,
    /// Draft of the own-app Discord client secret, preloaded from auth.json.
    auth_client_secret: String,
}

pub fn run() -> i32 {
    // Single-instance guard: a second GUI must not open over a running one.
    // The lock self-releases when this process exits.
    let _lock = match hyprlay_core::singleton::acquire(hyprlay_core::bins::GUI_LOCK) {
        Ok(lock) => lock,
        Err(AcquireError::AlreadyHeld) => {
            eprintln!("hyprlay-gui: another instance is already running");
            return 1;
        }
        Err(e) => {
            eprintln!("hyprlay-gui: {e}");
            return 1;
        }
    };

    iced::application(boot, update, view)
        .window(window_settings())
        // Fixed dark theme, Discord-flavored; never follows the system.
        .theme(theme_for)
        .subscription(subscribe)
        .run()
        .err()
        .map_or(0, |e| {
            eprintln!("gui failed to start: {e}");
            1
        })
}

/// The GUI window settings. `application_id` is a Linux-only (Wayland/X11)
/// `PlatformSpecific` field; other platforms get the default settings so the
/// winit window still builds.
fn window_settings() -> iced::window::Settings {
    iced::window::Settings {
        size: iced::Size::new(800.0, 620.0),
        min_size: Some(iced::Size::new(640.0, 480.0)),
        resizable: true,
        icon: Some(crate::platform::icon::window_icon()),
        platform_specific: platform_specific(),
        ..Default::default()
    }
}

/// The platform-specific window settings. `PlatformSpecific` is a different
/// struct per target: on Linux it carries the Wayland/X11 `application_id`;
/// elsewhere it is the winit variant and we keep the defaults.
fn platform_specific() -> iced::window::settings::PlatformSpecific {
    #[cfg(target_os = "linux")]
    {
        use iced::window::settings::PlatformSpecific;
        PlatformSpecific {
            application_id: hyprlay_core::bins::GUI_APP_ID.to_string(),
            ..Default::default()
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        iced::window::settings::PlatformSpecific::default()
    }
}

fn boot() -> (Gui, Task<Message>) {
    let saved_credentials = hyprlay_core::credentials::load();
    (
        Gui {
            config: config::load(),
            drafts: HashMap::new(),
            num_drafts: HashMap::new(),
            last_reply: String::new(),
            daemon_state: DaemonState::Connecting,
            auto_start: AutoStart::watching(),
            control: Arc::new(SystemControl),
            dirty: false,
            monitors: Vec::new(),
            section: Section::Position,
            search: String::new(),
            last_scroll_y: 0.0,
            picker: None,
            picker_drag: false,
            picker_pos: Point::ORIGIN,
            auth_client_id: saved_credentials
                .as_ref()
                .map(|c| c.client_id.clone())
                .unwrap_or_default(),
            auth_client_secret: saved_credentials
                .map(|c| c.client_secret)
                .unwrap_or_default(),
        },
        Task::batch([
            Task::perform(send(Command::Dump.to_string()), Message::Applied),
            Task::perform(
                async {
                    crate::platform::compositor::detect()
                        .monitors()
                        .into_iter()
                        .map(|m| m.name)
                        .collect()
                },
                Message::Monitors,
            ),
        ]),
    )
}

fn subscribe(_gui: &Gui) -> Subscription<Message> {
    Subscription::batch([
        iced::time::every(std::time::Duration::from_secs(2)).map(|_| Message::RefreshStatus),
        // Only "ignored" events reach us, so typing in a text field never
        // triggers shortcuts.
        keyboard::listen().map(Message::KeyPressed),
    ])
}

/// Blocking socket round-trip off the UI thread.
async fn send(command: String) -> String {
    tokio::task::spawn_blocking(move || {
        ctl::send_command_line(&crate::platform::ipc::control::Control, &command)
            .unwrap_or_else(|| "error: daemon unreachable".into())
    })
    .await
    .unwrap_or_else(|_| "error: command task failed".into())
}
