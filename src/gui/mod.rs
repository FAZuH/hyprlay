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
//! (sections) / scrollable content / labeled status bar (unsaved marker,
//! Start/Stop toggle, daemon state, last reply). Every field has a
//! tooltip, and the search box filters fields by label, tooltip, and
//! section name. Changes apply
//! live but are NOT persisted until "Save"; closing with unsaved changes is
//! allowed (the daemon keeps runtime state, but a daemon restart reverts to
//! config.toml).

mod daemon;
mod fields;
mod picker;
mod theme;

use std::collections::HashMap;
use std::sync::Arc;

use daemon::AutoStart;
use daemon::DaemonState;
use fields::SEARCH_ID;
use fields::Section;
use fields::search_page;
use fields::section_page;
use hyprlay_core::config::Config;
use hyprlay_core::config::HorizontalAnchor as H;
use hyprlay_core::config::PALETTES;
use hyprlay_core::config::VerticalAnchor as V;
use hyprlay_core::config::{self};
use hyprlay_core::credentials::AppCredentials;
use hyprlay_core::ctl;
use hyprlay_core::daemon_control::DaemonControl;
use hyprlay_core::daemon_control::StopPolicy;
use hyprlay_core::daemon_control::SystemControl;
use hyprlay_core::daemon_control::Toggle;
use hyprlay_core::domain::Command;
use hyprlay_core::domain::HexColor;
use hyprlay_core::domain::Key;
use hyprlay_core::domain::Value;
use hyprlay_core::domain::corner_of;
use hyprlay_core::singleton::AcquireError;
use iced::Alignment;
use iced::Element;
use iced::Length;
use iced::Point;
use iced::Subscription;
use iced::Task;
use iced::keyboard::key;
use iced::keyboard::{self};
use iced::widget::button;
use iced::widget::column;
use iced::widget::container;
use iced::widget::row;
use iced::widget::text;
use iced::widget::text_input;
use picker::ColorTarget;
use picker::apply_hue;
use picker::apply_sv;
use theme::AMBER;
use theme::BRIGHT;
use theme::HEADER_BG;
use theme::MUTED;
use theme::REPLY_GREEN;
use theme::SIDEBAR_BG;
use theme::nav_style;
use theme::panel;
use theme::plain_style;
use theme::primary_style;
use theme::theme_for;

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
    Section(Section),
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
    section: Section,
    search: String,
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
    let _lock = match hyprlay_core::singleton::acquire("hyprlay-gui") {
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
        .window(iced::window::Settings {
            size: iced::Size::new(800.0, 620.0),
            min_size: Some(iced::Size::new(640.0, 480.0)),
            resizable: true,
            ..Default::default()
        })
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
                    hyprlay_core::compositor::detect()
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
        ctl::send_command_line(&command).unwrap_or_else(|| "error: daemon unreachable".into())
    })
    .await
    .unwrap_or_else(|_| "error: command task failed".into())
}

/// Run one Start/Stop action (systemctl, sibling spawn, or socket quit) off
/// the UI thread, same blocking pattern as [`send`].
async fn run_toggle(control: Arc<dyn DaemonControl>, toggle: Toggle) -> Option<String> {
    tokio::task::spawn_blocking(move || {
        hyprlay_core::daemon_control::execute_toggle(&*control, toggle, StopPolicy::ViaSystemctl)
    })
    .await
    .unwrap_or_else(|e| Some(format!("error: daemon toggle task failed: {e}")))
}

/// Persist own-app credentials off the UI thread, then ask the daemon to
/// restart so it re-runs detect() and picks up the new backend. The
/// returned text lands in the status bar via [`Message::Applied`].
async fn apply_auth_credentials(creds: AppCredentials) -> String {
    // Read before the move: the decision text depends on what was applied.
    let cleared = creds.client_id.is_empty() && creds.client_secret.is_empty();
    let saved = tokio::task::spawn_blocking(move || hyprlay_core::credentials::save(&creds))
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e.to_string())));
    match saved {
        Ok(()) => {
            // The daemon's own reply only confirms delivery; the meaningful
            // text for the status bar is ours.
            let _ = send("restart".to_string()).await;
            if cleared {
                "credentials cleared, restarting daemon".to_string()
            } else {
                "credentials saved, restarting daemon".to_string()
            }
        }
        Err(e) => format!("error: could not write credentials: {e}"),
    }
}

fn update(gui: &mut Gui, message: Message) -> Task<Message> {
    match message {
        Message::Applied(reply) => {
            let reply = reply.trim().to_string();
            // Every reply is a potential probe outcome; only probe outcomes
            // actually move the state (see DaemonState::advance) — and
            // while the boot auto-start has the wheel, failures hold
            // `connecting…` instead of reporting the daemon dead.
            let launch = gui.auto_start.observe(&mut gui.daemon_state, &reply);
            // `dump` replies with the live runtime config as TOML — adopt it
            // so the GUI reflects unsaved daemon state. The [position]
            // header marks a dump; any other text is an ordinary reply. Any
            // in-flight input drafts are stale after an external reset, so
            // drop them too.
            if reply.contains("[position]") {
                if let Ok(live) = toml::from_str::<Config>(&reply) {
                    gui.config = live;
                    gui.drafts.clear();
                    gui.num_drafts.clear();
                }
            } else if reply == "saved" {
                gui.dirty = false;
            } else if !reply.is_empty() && !reply.starts_with("status=") {
                // status= replies are consumed by the state chip above;
                // everything else is ordinary status-bar traffic.
                gui.last_reply = reply;
            }
            match launch {
                Some(toggle) => {
                    // Opening the GUI brings the daemon up: fire-and-forget
                    // off the UI thread, through the same DaemonControl path
                    // as the Start button. This is also why closing the
                    // window never stops the daemon — nothing here ties its
                    // lifetime to GUI exit (systemctl owns the unit; the
                    // fallback spawn detaches into its own process group).
                    let control = Arc::clone(&gui.control);
                    Task::perform(run_toggle(control, toggle), Message::ToggleResult)
                }
                None => Task::none(),
            }
        }
        Message::RefreshStatus => {
            Task::perform(send(Command::Status.to_string()), Message::Applied)
        }
        Message::ToggleDaemon => {
            let Some(toggle) = gui.daemon_state.toggle() else {
                return Task::none();
            };
            let control = Arc::clone(&gui.control);
            Task::perform(run_toggle(control, toggle), Message::ToggleResult)
        }
        Message::ToggleResult(failure) => {
            // The boot bring-up attempt finished either way; stop holding
            // the connecting line on its behalf.
            gui.auto_start.settled();
            if let Some(text) = failure {
                gui.last_reply = text;
            }
            // Whether it worked is only visible through a fresh probe; do
            // not wait for the next 2 s tick.
            Task::perform(send(Command::Status.to_string()), Message::Applied)
        }
        Message::Monitors(monitors) => {
            gui.monitors = monitors;
            Task::none()
        }
        Message::Save => {
            gui.dirty = false;
            Task::perform(send(Command::Save.to_string()), Message::Applied)
        }
        Message::ClearChanges => {
            // Revert the daemon's runtime state to the on-disk config by
            // replaying only the fields that actually differ.
            let saved = config::load();
            let commands = revert_commands(&gui.config, &saved);
            gui.config = saved;
            gui.drafts.clear();
            gui.num_drafts.clear();
            gui.dirty = false;
            Task::batch(
                commands
                    .into_iter()
                    .map(|c| Task::perform(send(c.to_string()), Message::Applied)),
            )
        }
        Message::ResetAll => {
            mark_dirty(gui, &Command::ResetAll);
            Task::perform(send(Command::ResetAll.to_string()), Message::Applied).chain(
                Task::perform(send(Command::Dump.to_string()), Message::Applied),
            )
        }
        Message::ResetSection(section) => {
            // Sections without a config group have nothing to reset; the
            // GUI hides their button, so this arm is a defensive no-op.
            let Some(group) = section.group() else {
                return Task::none();
            };
            let command = Command::ResetGroup(group);
            mark_dirty(gui, &command);
            Task::perform(send(command.to_string()), Message::Applied).chain(Task::perform(
                send(Command::Dump.to_string()),
                Message::Applied,
            ))
        }
        // `monitor` is answered by the shell before apply_config runs, so
        // the mirror must be updated here or the chip highlight lags behind.
        Message::SwitchMonitor(target) => {
            let command = Command::Set(
                Key::Monitor,
                Value::Target(match &target {
                    None => hyprlay_core::domain::MonitorTarget::Active,
                    Some(name) => hyprlay_core::domain::MonitorTarget::Named(name.clone()),
                }),
            );
            gui.config.monitor = target;
            mark_dirty(gui, &command);
            Task::perform(send(command.to_string()), Message::Applied)
        }
        Message::Palette(index) => {
            let Some(p) = PALETTES.get(index) else {
                return Task::none();
            };
            let cmds = [
                Command::Set(Key::SpeakingColor, Value::Color(p.speaking)),
                Command::Set(Key::TextColor, Value::Color(p.text)),
                Command::Set(Key::BoxColor, Value::Color(p.box_bg)),
            ];
            for cmd in &cmds {
                cmd.clone().apply_config(&mut gui.config);
            }
            // All three palette entries are Sets: one decision covers them.
            mark_dirty(gui, &cmds[0]);
            Task::batch(
                cmds.into_iter()
                    .map(|c| Task::perform(send(c.to_string()), Message::Applied)),
            )
        }
        Message::Section(section) => {
            gui.section = section;
            Task::none()
        }
        Message::Search(query) => {
            gui.search = query;
            Task::none()
        }
        Message::KeyPressed(event) => shortcut(gui, event),
        Message::PickerToggle(target) => {
            gui.picker = if gui.picker == Some(target) {
                None
            } else {
                Some(target)
            };
            gui.picker_drag = false;
            Task::none()
        }
        // Color changes from every editor (hex field, RGB sliders, picker
        // drags) funnel through the same apply path. Invalid hex is kept as
        // a per-editor draft so the text input doesn't snap back mid-typing;
        // only valid values reach the mirror and the daemon.
        Message::ColorHex(target, hex) => match hex.parse::<HexColor>() {
            Ok(value) => {
                gui.drafts.remove(&target);
                ColorTarget::set_field(target, &mut gui.config, value);
                let command = target.command(value);
                mark_dirty(gui, &command);
                Task::perform(send(command.to_string()), Message::Applied)
            }
            Err(_) => {
                gui.drafts.insert(target, hex);
                Task::none()
            }
        },
        Message::NumText(key, raw) => match raw.trim().parse::<i64>() {
            // Valid and inside the daemon's bounds: commit immediately.
            // Anything else (empty, half-typed, out of range) stays as a
            // draft so the input doesn't snap back while typing.
            Ok(v) if num_in_bounds(key, v) => apply_num(gui, key, v),
            _ => {
                gui.num_drafts.insert(key, raw);
                Task::none()
            }
        },
        Message::NumDrag(key, v) => {
            let (min, max) = key.num_bounds().expect("slider keys are numeric");
            apply_num(gui, key, (v as i64).clamp(min, max))
        }
        Message::NumReset(key) => {
            let Value::Num(default) = key.value_of(&Config::default()) else {
                unreachable!("number_row only renders numeric keys");
            };
            apply_num(gui, key, default)
        }
        Message::ColorPart(target, part, v) => {
            let current = ColorTarget::field(target, &gui.config).rgb();
            let mut bytes = current;
            if let Some(slot) = bytes.get_mut(part as usize) {
                *slot = (v * 255.0).round() as u8;
            }
            let value = HexColor::from_rgb8(bytes[0], bytes[1], bytes[2]);
            update(gui, Message::ColorHex(target, value.to_string()))
        }
        Message::SvPress(target) => {
            gui.picker_drag = true;
            let p = gui.picker_pos;
            apply_sv(gui, target, p)
        }
        Message::SvMove(target, p) => {
            gui.picker_pos = p;
            if gui.picker_drag {
                apply_sv(gui, target, p)
            } else {
                Task::none()
            }
        }
        Message::HuePress(target) => {
            gui.picker_drag = true;
            let p = gui.picker_pos;
            apply_hue(gui, target, p)
        }
        Message::HueMove(target, p) => {
            gui.picker_pos = p;
            if gui.picker_drag {
                apply_hue(gui, target, p)
            } else {
                Task::none()
            }
        }
        Message::PickerRelease => {
            gui.picker_drag = false;
            Task::none()
        }
        Message::SetFlag(key, v) => {
            let command = Command::Set(key, Value::Flag(v));
            // The daemon decides persistence with its pre-apply autosave
            // value; capture ours before the optimistic mirror flips too.
            let persists = hyprlay_core::domain::should_persist(&command, gui.config.auto_save);
            command.clone().apply_config(&mut gui.config);
            if !persists {
                gui.dirty = true;
            }
            Task::perform(send(command.to_string()), Message::Applied)
        }
        Message::AuthClientId(id) => {
            gui.auth_client_id = id;
            Task::none()
        }
        Message::AuthClientSecret(secret) => {
            gui.auth_client_secret = secret;
            Task::none()
        }
        Message::AuthApply => {
            // Credentials deliberately bypass the ctl protocol (secrets
            // must never travel the socket): they go straight to auth.json,
            // and only an opaque "restart" crosses the socket afterwards.
            let creds = AppCredentials {
                client_id: gui.auth_client_id.trim().to_string(),
                client_secret: gui.auth_client_secret.trim().to_string(),
            };
            Task::perform(apply_auth_credentials(creds), Message::Applied)
        }
        command => {
            let command = command_for(command);
            mark_dirty(gui, &command);
            command.clone().apply_config(&mut gui.config);
            Task::perform(send(command.to_string()), Message::Applied)
        }
    }
}

/// Keyboard shortcuts: Ctrl+S save, Ctrl+R reset section, Ctrl+Shift+R
/// reset all, Ctrl+F search, Ctrl+1-5 sections, Escape clears the search.
fn shortcut(gui: &mut Gui, event: keyboard::Event) -> Task<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return Task::none();
    };
    if !modifiers.control() {
        if matches!(key, keyboard::Key::Named(key::Named::Escape)) && !gui.search.is_empty() {
            gui.search.clear();
        }
        return Task::none();
    }
    let keyboard::Key::Character(ch) = &key else {
        return Task::none();
    };
    match ch.to_lowercase().as_str() {
        "s" => update(gui, Message::Save),
        "r" if modifiers.shift() => update(gui, Message::ResetAll),
        "r" => update(gui, Message::ResetSection(gui.section)),
        "f" => iced_runtime::widget::operation::focus(widget_id()),
        _ => match ch.parse::<usize>() {
            // Ctrl+1..5 jump between sections.
            Ok(n) if (1..=Section::ALL.len()).contains(&n) => {
                update(gui, Message::Section(Section::at(n - 1).unwrap()))
            }
            _ => Task::none(),
        },
    }
}

fn widget_id() -> iced::widget::Id {
    iced::widget::Id::new(SEARCH_ID)
}

/// Commit one numeric value to the mirror and the daemon. The caller has
/// already made sure the value is inside the key's bounds.
fn apply_num(gui: &mut Gui, key: Key, value: i64) -> Task<Message> {
    gui.num_drafts.remove(&key);
    let command = Command::Set(key, Value::Num(value));
    mark_dirty(gui, &command);
    command.clone().apply_config(&mut gui.config);
    Task::perform(send(command.to_string()), Message::Applied)
}

/// Mirror of the daemon's persistence rule: a change is "unsaved" exactly
/// when the daemon would not have persisted it. Decided with the
/// pre-application autosave value — the same one the daemon uses — so
/// flipping auto-save itself never leaves a phantom badge.
fn mark_dirty(gui: &mut Gui, command: &Command) {
    if !hyprlay_core::domain::should_persist(command, gui.config.auto_save) {
        gui.dirty = true;
    }
}

fn num_in_bounds(key: Key, v: i64) -> bool {
    key.num_bounds()
        .is_some_and(|(min, max)| v >= min && v <= max)
}

/// Commands that bring `live` back to `saved`, one per differing key.
/// Used by "clear changes"; empty when there is nothing to revert. Walking
/// the shared [`Key`] table means a newly added setting can never be
/// forgotten here — it shows up in the diff the moment it exists.
fn revert_commands(live: &Config, saved: &Config) -> Vec<Command> {
    Key::ALL
        .into_iter()
        .filter(|k| k.value_of(live) != k.value_of(saved))
        .map(|k| Command::Set(k, k.value_of(saved)))
        .collect()
}

/// Turn a GUI interaction into its control-socket command. The local config
/// mirror is the same [`Command::apply_config`] the daemon runs, so both
/// sides can never disagree about what a setting change means.
fn command_for(message: Message) -> Command {
    match message {
        Message::Position(h, v) => Command::Set(Key::Position, Value::Corner(corner_of(h, v))),
        // Rides the generic apply path like Position: mirror locally, send
        // the same wire command the CLI would.
        Message::Anchor(mode) => Command::Set(Key::Anchor, Value::Anchor(mode)),
        Message::SetFlag(..) => unreachable!("flags are handled directly in update"),
        // Handled directly in `update`; unreachable here.
        Message::NumText(..)
        | Message::NumDrag(..)
        | Message::NumReset(_)
        | Message::ColorPart(..)
        | Message::ColorHex(..)
        | Message::PickerToggle(..)
        | Message::SvPress(..)
        | Message::SvMove(..)
        | Message::HuePress(..)
        | Message::HueMove(..)
        | Message::PickerRelease
        | Message::Palette(_)
        | Message::Section(_)
        | Message::Search(_)
        | Message::KeyPressed(_)
        | Message::Save
        | Message::ClearChanges
        | Message::ResetAll
        | Message::ResetSection(_)
        | Message::SwitchMonitor(_)
        | Message::Monitors(_)
        | Message::Applied(_)
        | Message::RefreshStatus
        | Message::ToggleDaemon
        | Message::ToggleResult(_)
        | Message::AuthClientId(_)
        | Message::AuthClientSecret(_)
        | Message::AuthApply => unreachable!("handled before command_for"),
    }
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

fn view(gui: &Gui) -> Element<'_, Message> {
    let content = if gui.search.trim().is_empty() {
        section_page(gui, gui.section)
    } else {
        search_page(gui)
    };

    column![
        header(gui),
        container(row![sidebar(gui), content]).height(Length::Fill),
        status_bar(gui),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// Title, search box, and global actions on the darkest strip.
fn header(gui: &Gui) -> Element<'_, Message> {
    // "Clear changes" only does something while the runtime config differs
    // from disk; a disabled press target communicates that at a glance.
    let mut clear = button(text("Clear changes")).style(plain_style());
    if gui.dirty {
        clear = clear.on_press(Message::ClearChanges);
    }
    container(
        row![
            text("hyprlay").size(14).color(BRIGHT),
            text_input("Search settings…  Ctrl+F", &gui.search)
                .id(widget_id())
                .on_input(Message::Search)
                .size(13)
                .padding([4, 8]),
            clear,
            button(text("Reset all"))
                .on_press(Message::ResetAll)
                .style(plain_style()),
            button(text("Save"))
                .on_press(Message::Save)
                .style(primary_style(gui.dirty)),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding([8, 12])
    .width(Length::Fill)
    .style(panel(HEADER_BG))
    .into()
}

/// Section navigation plus the shortcut cheat-sheet.
fn sidebar(gui: &Gui) -> Element<'_, Message> {
    let mut nav = column![].spacing(4);
    for (i, s) in Section::ALL.iter().enumerate() {
        let selected = gui.section == *s && gui.search.trim().is_empty();
        nav = nav.push(
            button(
                row![
                    text(s.name().to_string()).size(13),
                    iced::widget::Space::new().width(Length::Fill),
                    text(format!("Ctrl+{}", i + 1)).size(9).color(MUTED),
                ]
                .align_y(Alignment::Center)
                .width(Length::Fill),
            )
            .on_press(Message::Section(*s))
            .width(Length::Fill)
            .style(nav_style(selected)),
        );
    }
    let hints =
        "\nCtrl+S    save\nCtrl+R    reset section\nCtrl+F    search\nEsc       clear search";
    let col = column![
        nav,
        iced::widget::Space::new().height(Length::Fill),
        text(format!("shortcuts{hints}")).size(10).color(MUTED),
    ]
    .spacing(8);
    container(col)
        .width(Length::Fixed(160.0))
        .height(Length::Fill)
        .padding([10, 8])
        .style(panel(SIDEBAR_BG))
        .into()
}

fn status_bar(gui: &Gui) -> Element<'_, Message> {
    let unsaved = if gui.dirty {
        text("● unsaved").size(11).color(AMBER)
    } else {
        text("").size(11)
    };
    container(
        row![
            unsaved,
            daemon_toggle(gui),
            text("daemon").size(10).color(MUTED),
            text(brief_status(gui.daemon_state.text())).size(11),
            iced::widget::Space::new().width(Length::Fill),
            text("last change").size(10).color(MUTED),
            text(gui.last_reply.clone()).size(11).color(REPLY_GREEN),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding([6, 12])
    .width(Length::Fill)
    .style(panel(HEADER_BG))
    .into()
}

/// Bottom-left Start/Stop control. Disabled (no press target) while no
/// probe has answered yet, mirroring how "Clear changes" disables itself.
fn daemon_toggle(gui: &Gui) -> Element<'_, Message> {
    let mut toggle = button(text(gui.daemon_state.label()).size(11)).style(plain_style());
    if gui.daemon_state.toggle().is_some() {
        toggle = toggle.on_press(Message::ToggleDaemon);
    }
    toggle.into()
}

/// "status=connected channel=ngobrol 3 participants=2 …" →
/// "connected · ngobrol 3". Channel names may contain spaces, so slice on
/// the known field markers instead of words.
fn brief_status(full: &str) -> String {
    if !full.starts_with("status=") {
        return full.to_string();
    }
    let conn = full
        .strip_prefix("status=")
        .and_then(|rest| rest.split(' ').next())
        .unwrap_or("unknown");
    let channel = full.find("channel=").and_then(|start| {
        full[start..]
            .find(" participants=")
            .map(|end| &full[start + "channel=".len()..start + end])
    });
    match channel {
        Some(c) => format!("{conn} · {c}"),
        None => conn.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_matches_label_tip_and_section_name() {
        let field = fields::Field {
            section: Section::Position,
            label: "offset x",
            tip: "Horizontal distance in px from the anchored screen edge.",
            render: fields::f_offset_x,
        };
        assert!(fields::search_matches(&field, "offset"));
        assert!(fields::search_matches(&field, "horizontal"));
        assert!(fields::search_matches(&field, "POSITION"));
        assert!(!fields::search_matches(&field, "avatar"));
        assert!(!fields::search_matches(&field, ""));
    }

    #[test]
    fn sections_map_one_to_one_onto_config_groups() {
        use hyprlay_core::domain::Group;
        // Only config-backed sections participate; Connection has no group.
        let config_backed: Vec<_> = Section::ALL
            .into_iter()
            .filter_map(|section| section.group().map(|group| (section, group)))
            .collect();
        assert_eq!(config_backed.len(), Group::ALL.len());
        for ((section, group), expected) in config_backed.into_iter().zip(Group::ALL) {
            // The reset button sends one ResetGroup per GUI section; if a
            // section ever fails to map, its fields could never be reset.
            assert_eq!(group, expected);
            assert_eq!(
                format!("{expected}"),
                section.name().to_lowercase(),
                "section {} diverged from group {}",
                section.name(),
                group
            );
        }
    }

    /// Exactly one section is exempt from the reset machinery, and its name
    /// must stay stable because the shortcut hints and search rely on it.
    #[test]
    fn every_section_except_connection_maps_to_a_group() {
        for section in Section::ALL {
            match section.group() {
                Some(_) => assert_ne!(section.name(), "Connection"),
                None => assert_eq!(section.name(), "Connection"),
            }
        }
    }

    #[test]
    fn every_field_has_a_nonempty_tooltip() {
        for f in fields::FIELDS {
            assert!(!f.tip.is_empty(), "field {} needs a tooltip", f.label);
            assert!(!f.label.is_empty());
        }
    }

    #[test]
    fn every_section_has_fields() {
        for s in Section::ALL {
            assert!(
                fields::FIELDS.iter().any(|f| f.section == s),
                "section {} has no fields",
                s.name()
            );
        }
    }

    /// Click-through was removed; no field may render it again.
    #[test]
    fn click_through_is_gone_from_the_field_registry() {
        assert!(!fields::FIELDS.iter().any(|f| f.label.contains("click")));
    }

    #[test]
    fn anchor_field_is_registered_in_the_position_section() {
        let field = fields::FIELDS
            .iter()
            .find(|f| f.label == "anchor")
            .expect("anchor field registered");
        assert_eq!(field.section, Section::Position);
    }

    #[test]
    fn anchor_setting_roundtrips_through_apply_and_revert() {
        // The exact Command path the GUI's generic change pipeline drives.
        let mut live = Config::default();
        let pin_bottom = Command::Set(
            Key::Anchor,
            Value::Anchor(hyprlay_core::config::AnchorMode::Bottom),
        );
        pin_bottom.clone().apply_config(&mut live);
        assert_eq!(live.anchor, hyprlay_core::config::AnchorMode::Bottom);

        // Reverting mirrors what "clear changes" replays: read the saved
        // value back through the shared table and re-apply it.
        let saved = Config::default();
        let revert = Command::Set(Key::Anchor, Key::Anchor.value_of(&saved));
        revert.apply_config(&mut live);
        assert_eq!(live.anchor, saved.anchor);
    }

    #[test]
    fn brief_status_keeps_multiword_channel_names_intact() {
        let full = "status=connected channel=ngobrol 3 participants=2 rtl=on monitor=eDP-1";
        assert_eq!(brief_status(full), "connected · ngobrol 3");
    }

    #[test]
    fn brief_status_without_channel_falls_back_to_connection_word() {
        assert_eq!(brief_status("status=disconnected"), "disconnected");
        assert_eq!(brief_status("connecting…"), "connecting…");
    }

    #[test]
    fn key_sets_use_the_cli_wire_names() {
        use hyprlay_core::config::OFFSETS;
        assert_eq!(
            Command::Set(Key::Opacity, Value::Num(42)).to_string(),
            "set opacity 42"
        );
        assert_eq!(
            Command::Set(Key::OffsetX, Value::Num(-12)).to_string(),
            "set offset-x -12"
        );
        assert_eq!(
            Command::Set(Key::TalkingOnly, Value::Flag(true)).to_string(),
            "set talking-only on"
        );
        assert!(
            (OFFSETS.min as i64..=OFFSETS.max as i64).contains(&-12),
            "test value must stay inside the shared bounds"
        );
    }

    #[test]
    fn every_numeric_key_has_sane_bounds() {
        for key in Key::ALL {
            if let Some((min, max)) = key.num_bounds() {
                assert!(min <= max, "{} has an inverted range", key.name());
            } else {
                // Non-numeric keys must not pretend to have slider bounds.
                assert!(
                    !key.slider_bounds(&Config::default()).is_some()
                        || matches!(
                            key,
                            Key::OffsetX
                                | Key::OffsetY
                                | Key::Width
                                | Key::Scale
                                | Key::AvatarSize
                                | Key::TextSize
                                | Key::Spacing
                                | Key::MaxName
                                | Key::Opacity
                                | Key::AvatarOpacity
                                | Key::TextOpacity
                                | Key::BoxOpacity
                        ),
                    "{} renders a slider without numeric bounds",
                    key.name()
                );
            }
        }
    }

    #[test]
    fn revert_commands_do_nothing_when_configs_match() {
        let cfg = Config::default();
        assert!(revert_commands(&cfg, &cfg).is_empty());
    }

    #[test]
    fn revert_commands_cover_every_differing_key_once() {
        use hyprlay_core::config::HorizontalAnchor as H;
        use hyprlay_core::config::VerticalAnchor as V;
        // show_own_user defaults to true, so flipping it off is a real diff.
        let saved = Config {
            horizontal: H::Right,
            vertical: V::Top, // top-right corner
            rtl: true,
            offset_x: 40,
            opacity: 70,
            width: 500,
            show_own_user: false,
            monitor: Some("DP-2".into()),
            speaking_color: "#00ff00".parse().unwrap(),
            ..Config::default()
        };

        let cmds = revert_commands(&Config::default(), &saved);
        for expected in [
            "set position top-right",
            "set rtl on",
            "set offset-x 40",
            "set opacity 70",
            "set width 500",
            "set own-user off",
            "set monitor DP-2",
        ] {
            assert!(
                cmds.iter().any(|c| c.to_string() == expected),
                "missing revert command {expected}"
            );
        }
        assert!(
            cmds.iter()
                .any(|c| c.to_string().starts_with("set speaking-color "))
        );
        // Exactly one command per changed key — no redundant spam.
        assert_eq!(cmds.len(), 8, "unexpected extra commands: {cmds:?}");
    }
}
