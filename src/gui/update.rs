//! Update layer: the one flat [`update`] match — the app's dispatch
//! table — plus the keyboard [`shortcut`] dispatcher that feeds it and
//! the async effects its arms spawn off the UI thread.

use std::sync::Arc;

use hyprlay_core::config::Config;
use hyprlay_core::config::PALETTES;
use hyprlay_core::config::{self};
use hyprlay_core::credentials::AppCredentials;
use hyprlay_core::daemon_control::DaemonControl;
use hyprlay_core::daemon_control::StopPolicy;
use hyprlay_core::daemon_control::Toggle;
use hyprlay_core::domain::Command;
use hyprlay_core::domain::HexColor;
use hyprlay_core::domain::Key;
use hyprlay_core::domain::Value;
use hyprlay_core::status::StatusFields;
use iced::Task;
use iced::keyboard::key;
use iced::keyboard::{self};

use super::Gui;
use super::Message;
use super::commands::apply_num;
use super::commands::command_for;
use super::commands::mark_dirty;
use super::commands::num_in_bounds;
use super::commands::revert_commands;
use super::fields::Section;
use super::picker::ColorTarget;
use super::picker::apply_hue;
use super::picker::apply_sv;
use super::scroll::BOTTOM_SLACK;
use super::scroll::active_section_for;
use super::scroll::measure_sections;
use super::scroll::restore_scroll;
use super::scroll::scroll_to_section;
use super::scroll::widget_id;
use super::send;

pub(super) fn update(gui: &mut Gui, message: Message) -> Task<Message> {
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
            } else if !reply.is_empty() && !StatusFields::is_status_line(&reply) {
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
        Message::Navigate(section) => {
            // D3: jumping while searching first returns to the one-page
            // view; the `measure_sections` task below runs against the
            // layout built after this re-render, so its offsets are fresh.
            if !gui.search.trim().is_empty() {
                gui.search.clear();
            }
            // Immediate highlight — don't make the sidebar wait for the
            // measure round-trip.
            gui.section = section;
            measure_sections(Some(section))
        }
        Message::Scrolled(offset_y) => {
            // Continuously tracked so a search-clear can restore it (D4);
            // nothing reports Scrolled while the search page is up, so the
            // value freezes at its pre-search state.
            gui.last_scroll_y = offset_y;
            measure_sections(None)
        }
        Message::Measured {
            offsets,
            max_scroll,
            jump,
        } => match jump {
            Some(section) => scroll_to_section(section, offsets),
            None => {
                // Scrollspy: at the very end of the page the last header
                // can never reach the viewport top (Connection is shorter
                // than the viewport), so a bottomed-out scroll maps to
                // INFINITY and clamps to the last section. This branch
                // only sees a scrollable page: while the content fits its
                // viewport no scroll event fires, so max_scroll == 0 can
                // never get here.
                let at_end = gui.last_scroll_y >= max_scroll - BOTTOM_SLACK;
                let scroll_y = if at_end {
                    f32::INFINITY
                } else {
                    gui.last_scroll_y
                };
                gui.section = active_section_for(scroll_y, &offsets);
                Task::none()
            }
        },
        Message::Search(query) => {
            // D4: emptying the search re-shows the one-pager; land it back
            // on the offset tracked before the search began.
            let restore = !gui.search.trim().is_empty() && query.trim().is_empty();
            gui.search = query;
            if restore {
                restore_scroll(gui)
            } else {
                Task::none()
            }
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
            if key == Key::ShowOnFullscreen {
                gui.config.show_on_fullscreen = v;
                mark_dirty(gui, &command);
                Task::perform(send(command.to_string()), Message::Applied)
            } else {
                // The daemon decides persistence with its pre-apply autosave
                // value; capture ours before the optimistic mirror flips too.
                let persists = hyprlay_core::domain::should_persist(&command, gui.config.auto_save);
                command.clone().apply_config(&mut gui.config);
                if !persists {
                    gui.dirty = true;
                }
                Task::perform(send(command.to_string()), Message::Applied)
            }
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
/// reset all, Ctrl+F search, Ctrl+1..5 jumps to section N (the same path
/// as a sidebar click), Escape clears the search.
fn shortcut(gui: &mut Gui, event: keyboard::Event) -> Task<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return Task::none();
    };
    if !modifiers.control() {
        if matches!(key, keyboard::Key::Named(key::Named::Escape)) && !gui.search.trim().is_empty()
        {
            gui.search.clear();
            // D4: Esc empties the search, so land the one-pager back on
            // its pre-search offset.
            return restore_scroll(gui);
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
            // Ctrl+1..5 scroll the one-pager to the section's header.
            Ok(n) if (1..=Section::ALL.len()).contains(&n) => {
                update(gui, Message::Navigate(Section::at(n - 1).unwrap()))
            }
            _ => Task::none(),
        },
    }
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use iced::Point;

    use super::*;
    use crate::gui::daemon::AutoStart;
    use crate::gui::daemon::DaemonState;

    /// D3: a sidebar click or Ctrl+1..5 while a search is up first drops
    /// the query (returning to the one-page view) and shows the target
    /// section's highlight immediately, without waiting for the measure
    /// round-trip.
    #[test]
    fn navigating_while_searching_clears_the_search_and_sets_the_section() {
        let mut gui = gui_with_search("avatar");
        // The returned Task carries the measure-then-jump round-trip; the
        // state transition is what is asserted here.
        let _ = update(&mut gui, Message::Navigate(Section::Colors));
        assert!(gui.search.is_empty());
        assert_eq!(gui.section, Section::Colors);
    }

    /// D4 mechanism: the restore on search-clear scrolls back to whatever
    /// offset the scrollspy last tracked, so scrolling must keep that value
    /// current, and emptying the search must not clobber it.
    #[test]
    fn scrolling_tracks_the_offset_that_the_search_restore_will_use() {
        let mut gui = gui_with_search("dim");
        // Both transitions return layout/scroll Tasks; only the tracked
        // state matters here.
        let _ = update(&mut gui, Message::Scrolled(412.5));
        assert!((gui.last_scroll_y - 412.5).abs() < f32::EPSILON);

        let _ = update(&mut gui, Message::Search(String::new()));
        assert!(gui.search.is_empty());
        assert!((gui.last_scroll_y - 412.5).abs() < f32::EPSILON);
    }

    /// Minimal `Gui` for state-transition tests: `boot()` touches the real
    /// config file, so build the struct with test values instead. Only the
    /// navigation fields matter here.
    fn gui_with_search(query: &str) -> Gui {
        Gui {
            config: Config::default(),
            drafts: HashMap::new(),
            num_drafts: HashMap::new(),
            last_reply: String::new(),
            daemon_state: DaemonState::Connecting,
            auto_start: AutoStart::watching(),
            control: Arc::new(crate::platform::service::SystemControl),
            dirty: false,
            monitors: Vec::new(),
            section: Section::Position,
            search: query.to_string(),
            last_scroll_y: 0.0,
            picker: None,
            picker_drag: false,
            picker_pos: Point::ORIGIN,
            auth_client_id: String::new(),
            auth_client_secret: String::new(),
        }
    }
}
