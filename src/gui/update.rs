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
use hyprlay_core::domain::Reply;
use hyprlay_core::domain::Value;
use hyprlay_core::status::StatusFields;
use iced::Task;
use iced::keyboard::key;
use iced::keyboard::{self};
use iced_runtime::widget::operation;

use super::FocusTarget;
use super::Gui;
use super::Message;
use super::commands::apply_num;
use super::commands::command_for;
use super::commands::mark_dirty;
use super::commands::num_in_bounds;
use super::commands::revert_commands;
use super::commands::step_command;
use super::fields;
use super::fields::Section;
use super::picker::ColorTarget;
use super::picker::apply_hue;
use super::picker::apply_sv;
use super::scroll::BOTTOM_SLACK;
use super::scroll::Jump;
use super::scroll::active_section_for;
use super::scroll::measure_sections;
use super::scroll::restore_scroll;
use super::scroll::scroll_content_to;
use super::scroll::scroll_to_section;
use super::scroll::widget_id;
use super::send;

pub(super) fn update(gui: &mut Gui, message: Message) -> Task<Message> {
    match message {
        Message::Applied(reply) => {
            let reply = reply.trimmed();
            // Every reply is a potential probe outcome; only probe outcomes
            // actually move the state (see DaemonState::advance) — and
            // while the boot auto-start has the wheel, failures hold
            // `connecting…` instead of reporting the daemon dead.
            let launch = gui.auto_start.observe(&mut gui.daemon_state, &reply);
            // `dump` replies with the live runtime config as TOML — adopt it
            // so the GUI reflects unsaved daemon state. Any in-flight input
            // drafts are stale after an external reset, so drop them too.
            if reply.is_config_dump() {
                if let Ok(live) = toml::from_str::<Config>(reply.text()) {
                    gui.config = live;
                    gui.drafts.clear();
                    gui.num_drafts.clear();
                }
            } else if reply.text() == "saved" {
                gui.dirty = false;
            } else if !reply.text().is_empty() && !StatusFields::is_status_line(reply.text()) {
                // status= replies are consumed by the state chip above;
                // everything else, successes and failures alike, is
                // ordinary status-bar traffic. The colour comes from the
                // variant in view, so a failure paints as one.
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
                gui.last_reply = Reply::Error(text);
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
            measure_sections(Some(Jump::Section(section)))
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
            // A narrower query drops rows, so the focused one may be among
            // them: the ring is drawn where the page renders a row carrying
            // that key, and `activate_focus` reads `gui.focus` directly. Left
            // alone, nothing is ringed anywhere and Enter still fires the
            // command for a row that is off screen.
            if gui.focus.is_some_and(|t| !tab_order(gui).contains(&t)) {
                gui.focus = None;
            }
            if restore {
                restore_scroll(gui)
            } else {
                Task::none()
            }
        }
        // A programmatic scroll never fires `on_scroll`, so the GUI's own idea
        // of where the page is would stay where the last user scroll left it:
        // the search-clear restore needs the offset the user actually left
        // from, so a reveal records it here. It deliberately does not re-run
        // the scrollspy — a target past the end of the page clamps to the end
        // and reads as "scrolled to the end" (see `move_focus`). The highlight
        // belongs to the focus that asked for the reveal; the scrollspy still
        // tracks the user's own scrolling.
        Message::ScrollContentTo(y) => {
            // A search-page reveal scrolls the search results, not the
            // one-pager, so it must not become the one-pager's restore point.
            if gui.search.trim().is_empty() {
                gui.last_scroll_y = y;
            }
            scroll_content_to(y)
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
        // Enter inside the input: commit what is typed. A draft only ever holds
        // text `NumText` refused, so this is where that refusal is either stood
        // down or answered with the daemon's own wording.
        Message::NumSubmit(key) => commit_num(gui, key).unwrap_or_else(Task::none),
        // Escape that the input swallowed: it dropped its own focus, and the
        // row's half-typed value goes with it — the row shows its real value
        // again and nothing is applied.
        Message::EscapeCaptured => {
            if let Some(FocusTarget::Field(key)) = gui.focus {
                gui.num_drafts.remove(&key);
            }
            release_typing()
        }
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
    // Tab / Shift+Tab move focus, Enter / Space activate it. Iced's button
    // widget handles no keyboard events at all, so this is the only path a
    // keyboard-only user has; see FocusTarget for why the focus concept lives
    // in this app rather than in the framework.
    if matches!(key, keyboard::Key::Named(key::Named::Tab)) {
        return tab_out(gui, modifiers.shift());
    }
    if matches!(
        key,
        keyboard::Key::Named(key::Named::Enter) | keyboard::Key::Named(key::Named::Space)
    ) {
        return activate_focus(gui);
    }
    if !modifiers.control() {
        if matches!(key, keyboard::Key::Named(key::Named::Escape)) && !gui.search.trim().is_empty()
        {
            gui.search.clear();
            // D4: Esc empties the search, so land the one-pager back on
            // its pre-search offset.
            return restore_scroll(gui);
        }
        // An arrow steps the focused row, and arrows are free to: iced spends one on
        // the text input that holds real focus, so reaching here means no input
        // owns typing and the focused row is the only thing an arrow could
        // mean. (Iced's slider takes Up and Down of its own, but only while the
        // cursor is over it — a mouse user, not a keyboard one.) Right and Up
        // step forward, Left and Down back, and which step that is belongs to
        // the row: see `step_row`.
        return match &key {
            keyboard::Key::Named(key::Named::ArrowRight | key::Named::ArrowUp) => {
                step_row(gui, true)
            }
            keyboard::Key::Named(key::Named::ArrowLeft | key::Named::ArrowDown) => {
                step_row(gui, false)
            }
            _ => Task::none(),
        };
    }
    let keyboard::Key::Character(ch) = &key else {
        return Task::none();
    };
    match ch.to_lowercase().as_str() {
        "s" => update(gui, Message::Save),
        "r" if modifiers.shift() => update(gui, Message::ResetAll),
        "r" => update(gui, Message::ResetSection(gui.section)),
        "f" => operation::focus(widget_id()),
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
async fn apply_auth_credentials(creds: AppCredentials) -> Reply {
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
                Reply::Ok("credentials cleared, restarting daemon".into())
            } else {
                Reply::Ok("credentials saved, restarting daemon".into())
            }
        }
        Err(e) => Reply::Error(format!("error: could not write credentials: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D3: a sidebar click or Ctrl+1..5 while a search is up first drops
    /// the query and shows the target section's highlight immediately.
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

    /// A reveal on the search page must not overwrite the one-pager's tracked
    /// offset: `restore_scroll` would then land the one-pager at an offset that
    /// belonged to the search results instead of where the user left it.
    #[test]
    fn a_reveal_while_searching_leaves_the_one_pager_offset_alone() {
        let mut gui = gui_with_search("avatar");
        // A non-zero sentinel, so "unchanged" cannot pass by accident.
        gui.last_scroll_y = 412.5;
        let _ = update(&mut gui, Message::ScrollContentTo(640.0));
        assert_eq!(
            gui.last_scroll_y, 412.5,
            "a search-content offset is not the one-pager's restore point"
        );
    }

    /// Minimal `Gui` for state-transition tests: `boot()` touches the real
    /// config file, so build the struct with test values instead. Only the
    /// navigation fields matter here.
    fn gui_with_search(query: &str) -> Gui {
        crate::gui::test_gui(query)
    }
}

/// The tab order, derived from the visual order at `view.rs`
/// (`column![header, row![sidebar, content], status_bar]`): header actions,
/// then the sidebar nav items, then the content rows in the order
/// [`fields::FIELDS`] renders them, then the status bar's toggle.
///
/// One place, not one list per control. A new field lands in `FIELDS` and is
/// reachable without touching this. The order is `FIELDS`, not `Key::ALL`:
/// the rows are grouped by section while `Key::ALL` is grouped by wire order,
/// so walking `Key::ALL` puts `monitor` (the last Position row) third and
/// `rtl` (the third) eighth, and Tab ping-pongs the viewport. It is
/// `rendered_targets`, not `Key::ALL`, so the two credential rows are walked
/// in their own right — they edit no key, and the walk has to reach them all
/// the same.
fn tab_order(gui: &Gui) -> Vec<FocusTarget> {
    let mut out = vec![
        FocusTarget::ClearChanges,
        FocusTarget::ResetAll,
        FocusTarget::Save,
    ];
    out.extend((0..Section::ALL.len()).map(FocusTarget::Nav));
    out.extend(fields::rendered_targets(&gui.search));
    out.push(FocusTarget::ToggleDaemon);
    out
}

/// Tab / Shift+Tab: move focus to the next or previous target in the tab
/// order, wrapping at both ends. `None` currently held starts from the top.
fn move_focus(gui: &mut Gui, backwards: bool) -> Task<Message> {
    let order = tab_order(gui);
    let next = match gui.focus {
        None => {
            if backwards {
                order.last().copied()
            } else {
                order.first().copied()
            }
        }
        Some(current) => {
            let idx = order.iter().position(|t| *t == current);
            match idx {
                Some(i) => {
                    let n = order.len();
                    let next = if backwards {
                        (i + n - 1) % n
                    } else {
                        (i + 1) % n
                    };
                    order.get(next).copied()
                }
                // Focus was on something no longer in the order (a field the
                // search page dropped); start from the top.
                None => order.first().copied(),
            }
        }
    };
    gui.focus = next;
    // Tab walks the field rows down a one-page scroll, so landing on a row
    // has to bring it into view; a row already on screen leaves the page alone.
    match next {
        Some(target @ (FocusTarget::Field(_) | FocusTarget::Credential(_))) => {
            // The ring is on that row, so the sidebar names its section and
            // Ctrl+R resets that one. The scroll position cannot say which: a row
            // in the page's last screenful has a reveal target past `max_scroll`,
            // the scrollable clamps it to the end, and that reads as "scrolled to
            // its end" — which names Connection, the one section with no config
            // group, so Ctrl+R would reset nothing.
            if let Some(section) = fields::section_of(target) {
                gui.section = section;
            }
            // Iced 0.14 still owns typing: a `text_input` is the one widget
            // that has real keyboard focus, and a credential row is one. So
            // landing on it hands typing over — the same operation Ctrl+F uses
            // for the search box — and Tab away hands it back by focusing
            // nothing, which is what unfocuses the input. Without the second
            // half, Tab would leave focus off the row and typing would go on
            // editing the credential, which is the bug this half prevents.
            let typing = match target {
                FocusTarget::Credential(credential) => operation::focus(credential.input_id()),
                _ => release_typing(),
            };
            let reveal = fields::row_id(target).map(|id| measure_sections(Some(Jump::Row(id))));
            match reveal {
                Some(reveal) => Task::batch([typing, reveal]),
                None => typing,
            }
        }
        // Focus left the rows for chrome, or was dropped entirely. Same
        // hand-back, so no credential is left holding typing the ring has
        // moved off; a no-op when nothing was focused, which is every Tab
        // that never touched a credential.
        _ => release_typing(),
    }
}

/// Drop iced's own focus, so typing goes nowhere instead of into whatever text
/// input the ring has walked away from. `operation::focus` only does this as a
/// side effect of focusing something else, so a Tab that lands on chrome after
/// a credential needs it said outright. Iced keeps no public
/// `operation::unfocus`, but the operation itself is reachable through the
/// re-exported core.
fn release_typing() -> Task<Message> {
    iced_runtime::task::widget(iced_runtime::core::widget::operation::focusable::unfocus())
}

/// Enter / Space: activate the focused control. A disabled control takes
/// focus but is a no-op, mirroring how it drops its press target.
fn activate_focus(gui: &mut Gui) -> Task<Message> {
    let Some(target) = gui.focus else {
        return Task::none();
    };
    match target {
        FocusTarget::ClearChanges => update(gui, Message::ClearChanges),
        FocusTarget::ResetAll => update(gui, Message::ResetAll),
        FocusTarget::Save => update(gui, Message::Save),
        FocusTarget::Nav(i) => match Section::at(i) {
            Some(section) => update(gui, Message::Navigate(section)),
            None => Task::none(),
        },
        FocusTarget::ToggleDaemon => update(gui, Message::ToggleDaemon),
        // Typing already went into the input: `move_focus` handed iced's focus
        // to it when the row took focus, so the keystrokes were never ours and
        // there is nothing for Enter here to do. Committing the pair is the
        // "apply connection" button's job, deliberately mouse-only like the
        // per-section resets.
        FocusTarget::Credential(_) => Task::none(),
        FocusTarget::Field(key) => {
            // A bare Space/Enter on a flag flips it, which is the same thing
            // the bare `set <key>` form does on the wire.
            if matches!(key.parse_value(None), Ok(Value::Cycle))
                && let Value::Flag(current) = key.value_of(&gui.config)
            {
                return update(gui, Message::SetFlag(key, !current));
            }
            // A number row moves the keyboard into its input: the row's value
            // has arrows, and Enter is how a value gets typed instead. This is
            // the same hand-off `move_focus` makes for a credential row and the
            // same operation Ctrl+F uses for the search box, because it is the
            // one thing that makes typing go to a chosen widget at all.
            if key.num_bounds().is_some() {
                return operation::focus(fields::num_input_id(key));
            }
            // Any other row that offers a fixed set of choices is an option
            // select, and Enter activates it by stepping forward — the same
            // step the arrows take. What is left rings but stays inert: the
            // colour editors edit no number and offer no choices, and nothing
            // walks a focus chain for iced's widgets to be handed. Enter into
            // them is the next pass.
            step_row(gui, true)
        }
    }
}

/// One arrow press on the focused row. The row's own kind decides what an arrow
/// means on it, and the two kinds cannot overlap: a row that holds a number
/// offers no choices, and a row that offers choices holds no number (see
/// `commands::step_command`, which is where each row's step is derived). So
/// one key has exactly one owner per row, and neither path has to know the
/// other exists.
///
/// Focus does not move: the row keeps the ring the whole way, because the ring
/// is on the row and not on the control inside it.
fn step_row(gui: &mut Gui, forward: bool) -> Task<Message> {
    let Some(FocusTarget::Field(key)) = gui.focus else {
        return Task::none();
    };
    let Some(Command::Set(_, value)) = step_command(gui, key, forward) else {
        return Task::none();
    };
    match value {
        // `set monitor` is answered by the daemon shell rather than by config
        // application, and it is the one option row whose local mirror its own
        // message has to write.
        Value::Target(_) => update(gui, Message::SwitchMonitor(fields::monitor_name(&value))),
        // A number applies through the shared numeric path, which also drops
        // the row's draft: an arrow over a row whose input still holds refused
        // text leaves the input showing the value the arrow just set.
        Value::Num(next) => apply_num(gui, key, next),
        // Every other choice row rides the generic apply path its chip click
        // takes.
        _ => update(gui, Message::SetOption(key, value)),
    }
}

/// Tab / Shift+Tab: move the ring along the tab order — except out of a number
/// row that holds typed text, which swallows the Tab and commits that text
/// instead, because committing is what Tab means in the middle of a typed
/// value. A commit the bounds refuse keeps the caret in the input, so it keeps
/// the ring too; a row with nothing pending moves as every other target does.
fn tab_out(gui: &mut Gui, backwards: bool) -> Task<Message> {
    match gui.focus {
        Some(FocusTarget::Field(key)) if holds_typed_text(gui, key) => {
            commit_num(gui, key).unwrap_or_else(Task::none)
        }
        _ => move_focus(gui, backwards),
    }
}

/// Whether a number row holds text the input refused. `Message::NumText`
/// applies every value the bounds allow the moment it is typed, so a draft can
/// only ever be text that was *not* applied — which makes this the same thing
/// as "the keyboard is typing into this row", the one question Tab has to ask
/// before it can mean "commit".
fn holds_typed_text(gui: &Gui, key: Key) -> bool {
    key.num_bounds().is_some() && gui.num_drafts.contains_key(&key)
}

/// Commit what is typed in a number row: apply it when it is a value the bounds
/// allow, and hand typing back to the row either way, because the ring never
/// left it.
///
/// `None` is a refusal: the daemon's own parse error becomes the reply, the
/// row keeps the value it had, and the caret stays in the input for the user to
/// fix. No draft means nothing is pending — this is Enter on an untouched row —
/// so the only work left is the hand-back.
fn commit_num(gui: &mut Gui, key: Key) -> Option<Task<Message>> {
    let Some(raw) = gui.num_drafts.get(&key).cloned() else {
        return Some(release_typing());
    };
    match raw
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|v| num_in_bounds(key, *v))
    {
        Some(value) => Some(apply_num(gui, key, value)),
        None => {
            // The domain's own wording for this refusal, read off the same
            // parser the daemon uses, rather than a second message invented here.
            gui.last_reply = Reply::Error(
                key.parse_value(None)
                    .expect_err("a numeric key needs a value"),
            );
            None
        }
    }
}

#[cfg(test)]
mod focus_tests {
    use hyprlay_core::config::AnchorMode;

    use super::*;
    use crate::gui::Credential;
    use crate::gui::FocusTarget;

    /// A `Gui` with nothing focused and a clean config.
    fn gui() -> Gui {
        crate::gui::test_gui("")
    }

    /// Tab from nothing lands on the first target in the order; Tab again
    /// advances; Shift+Tab reverses.
    #[test]
    fn tab_moves_focus_through_the_order() {
        let mut g = gui();
        let _ = update(
            &mut g,
            Message::KeyPressed(no_key(keyboard::Key::Named(keyboard::key::Named::Tab))),
        );
        assert_eq!(g.focus, Some(FocusTarget::ClearChanges));

        let _ = update(
            &mut g,
            Message::KeyPressed(no_key(keyboard::Key::Named(keyboard::key::Named::Tab))),
        );
        assert_eq!(g.focus, Some(FocusTarget::ResetAll));

        let _ = update(&mut g, Message::KeyPressed(shift_tab()));
        assert_eq!(g.focus, Some(FocusTarget::ClearChanges));
    }

    /// Shift+Tab from nothing lands on the last target in the order.
    #[test]
    fn shift_tab_from_nothing_lands_on_the_last_target() {
        let mut g = gui();
        let _ = update(&mut g, Message::KeyPressed(shift_tab()));
        assert_eq!(g.focus, Some(FocusTarget::ToggleDaemon));
    }

    /// Tab wraps at both ends.
    #[test]
    fn tab_wraps() {
        let mut g = gui();
        g.focus = Some(FocusTarget::ToggleDaemon);
        let _ = update(
            &mut g,
            Message::KeyPressed(no_key(keyboard::Key::Named(keyboard::key::Named::Tab))),
        );
        assert_eq!(g.focus, Some(FocusTarget::ClearChanges));

        g.focus = Some(FocusTarget::ClearChanges);
        let _ = update(&mut g, Message::KeyPressed(shift_tab()));
        assert_eq!(g.focus, Some(FocusTarget::ToggleDaemon));
    }

    /// Enter activates the focused control: Save sets dirty false and sends.
    #[test]
    fn enter_activates_the_focused_control() {
        let mut g = gui();
        g.focus = Some(FocusTarget::Save);
        let _ = update(
            &mut g,
            Message::KeyPressed(no_key(keyboard::Key::Named(keyboard::key::Named::Enter))),
        );
        assert!(!g.dirty);
    }

    /// Enter on a cycle-able field flips it, which is the same thing the
    /// bare `set <key>` wire form does.
    #[test]
    fn enter_flips_a_cycle_able_field() {
        let mut g = gui();
        g.focus = Some(FocusTarget::Field(Key::Rtl));
        let before = g.config.rtl;
        let _ = update(
            &mut g,
            Message::KeyPressed(no_key(keyboard::Key::Named(keyboard::key::Named::Space))),
        );
        assert_eq!(g.config.rtl, !before);
    }

    /// Enter on a number row hands typing to that row's input instead of moving
    /// the value: a `text_input` is the one widget with real keyboard focus, so
    /// this is the operation that puts a caret there, and the value moves only
    /// once something is typed or an arrow says so. Read off the returned
    /// `Task`, which is the only thing a unit test can see of a widget
    /// operation, and compared against a colour row — the other kind of
    /// row with no cycle-able form, which has no input to enter at all.
    #[test]
    fn enter_on_a_number_row_hands_typing_to_its_input() {
        let mut g = gui();
        g.focus = Some(FocusTarget::Field(Key::Opacity));
        let before = g.config.opacity;
        let task = update(
            &mut g,
            Message::KeyPressed(no_key(keyboard::Key::Named(key::Named::Enter))),
        );
        assert_eq!(
            g.config.opacity, before,
            "Enter on its own changes no value"
        );
        assert_eq!(
            task.units(),
            1,
            "Enter on a number row runs the focus hand-off and nothing else"
        );

        let mut g = gui();
        g.focus = Some(FocusTarget::Field(Key::SpeakingColor));
        let task = update(
            &mut g,
            Message::KeyPressed(no_key(keyboard::Key::Named(key::Named::Enter))),
        );
        assert_eq!(task.units(), 0, "a colour editor has no input to enter");
    }

    /// The option selects: Enter and Right move to the next choice and Left to
    /// the previous one, and the ring stays on the row for all three — the
    /// choice is what moves, not the focus.
    #[test]
    fn enter_and_right_step_forward_and_left_steps_back() {
        for key in [
            keyboard::Key::Named(key::Named::Enter),
            keyboard::Key::Named(key::Named::ArrowRight),
        ] {
            let mut g = gui();
            g.focus = Some(FocusTarget::Field(Key::Anchor));
            let task = update(&mut g, Message::KeyPressed(no_key(key.clone())));
            assert_eq!(
                g.config.anchor,
                AnchorMode::Top,
                "{key:?} from auto lands on top"
            );
            assert_eq!(
                g.focus,
                Some(FocusTarget::Field(Key::Anchor)),
                "{key:?} moves the choice, not the focus"
            );
            assert_eq!(
                task.units(),
                1,
                "{key:?} sends exactly one command to the daemon"
            );
        }

        // Left walks the same row backwards, from where it already stands.
        let mut g = gui();
        g.focus = Some(FocusTarget::Field(Key::Anchor));
        g.config.anchor = AnchorMode::Bottom;
        let task = update(&mut g, Message::KeyPressed(arrow_left()));
        assert_eq!(g.config.anchor, AnchorMode::Top, "Left from bottom is top");
        assert_eq!(task.units(), 1, "and it sends one command");
    }

    /// Both ends stop, and "nothing happens" is asserted as nothing spawned: a
    /// boundary step must not put a command on the socket. Held arrows run the
    /// same step, so a key the user leans on at the last option stays there
    /// rather than running past it.
    #[test]
    fn a_step_at_either_end_stops_where_the_row_is() {
        let mut g = gui();
        g.focus = Some(FocusTarget::Field(Key::Anchor));

        let first = update(&mut g, Message::KeyPressed(arrow_left()));
        assert_eq!(
            g.config.anchor,
            AnchorMode::Auto,
            "Left on auto does not wrap to bottom"
        );
        assert_eq!(first.units(), 0, "and puts no command on the socket");

        // Three steps from auto reach bottom; four more, held, must not move.
        for _ in 0..3 {
            let _ = update(&mut g, Message::KeyPressed(arrow_right()));
        }
        assert_eq!(g.config.anchor, AnchorMode::Bottom);
        let held = update(&mut g, Message::KeyPressed(held_right()));
        assert_eq!(
            g.config.anchor,
            AnchorMode::Bottom,
            "a held Right at the last option stops there"
        );
        assert_eq!(held.units(), 0, "and sends nothing while held");
    }

    /// A held key is a stream of further KeyPressed events, and one repeat is
    /// one step. X11 auto-repeat is how a keyboard user holds an arrow, so a
    /// repeat that did nothing would leave held keys worse than tapped ones.
    #[test]
    fn a_held_arrow_repeats_the_step() {
        let mut g = gui();
        g.focus = Some(FocusTarget::Field(Key::Anchor));
        let _ = update(&mut g, Message::KeyPressed(held_right()));
        let _ = update(&mut g, Message::KeyPressed(held_right()));
        assert_eq!(
            g.config.anchor,
            AnchorMode::Bottom,
            "two repeats from auto land on bottom"
        );
    }

    /// The arrows are inert anywhere they cannot mean a step: a chrome button,
    /// a credential row (where iced's own focus owns them, to move the caret)
    /// and a colour editor, which edits no number and offers no choices.
    #[test]
    fn the_arrows_do_nothing_where_there_is_no_step() {
        for target in [
            FocusTarget::Save,
            FocusTarget::Nav(0),
            FocusTarget::Credential(Credential::ClientId),
            FocusTarget::Field(Key::SpeakingColor),
        ] {
            let mut g = gui();
            g.focus = Some(target);
            for key in [arrow_left(), arrow_right(), arrow_up(), arrow_down()] {
                let task = update(&mut g, Message::KeyPressed(key));
                assert_eq!(task.units(), 0, "{target:?} must ignore the arrows");
            }
        }
    }

    /// A number row steps with all four arrows, up and right forward, down and
    /// left back, by the row's own step — and the ring stays on the row, since
    /// it is the row that is focused and not the slider inside it.
    #[test]
    fn the_arrows_step_a_number_row() {
        let forward = [arrow_right(), arrow_up()];
        let back = [arrow_left(), arrow_down()];
        for key in forward {
            let mut g = gui();
            g.focus = Some(FocusTarget::Field(Key::Spacing));
            let task = update(&mut g, Message::KeyPressed(key.clone()));
            assert_eq!(g.config.spacing, 5, "{key:?} raises the value by one");
            assert_eq!(
                g.focus,
                Some(FocusTarget::Field(Key::Spacing)),
                "{key:?} moves the value, not the focus"
            );
            assert_eq!(task.units(), 1, "{key:?} sends exactly one command");
        }
        for key in back {
            let mut g = gui();
            g.focus = Some(FocusTarget::Field(Key::Spacing));
            let task = update(&mut g, Message::KeyPressed(key.clone()));
            assert_eq!(g.config.spacing, 3, "{key:?} lowers the value by one");
            assert_eq!(task.units(), 1, "{key:?} sends exactly one command");
        }
        // A negative value steps the same way and keeps its sign.
        let mut g = gui();
        g.config.offset_x = -12;
        g.focus = Some(FocusTarget::Field(Key::OffsetX));
        let _ = update(&mut g, Message::KeyPressed(arrow_right()));
        assert_eq!(
            g.config.offset_x, -11,
            "offsets cross zero the ordinary way"
        );
    }

    /// At a bound the value stops and nothing is sent: no write, no wire text.
    /// Held arrows run the same step, so leaning on one at the end of the range
    /// holds the value there instead of running past it. `opacity` is 100 on a
    /// clean config, which is its upper bound, so the very first press is the
    /// boundary.
    #[test]
    fn a_number_row_stops_at_its_bounds() {
        let mut g = gui();
        g.focus = Some(FocusTarget::Field(Key::Opacity));
        assert_eq!(g.config.opacity, 100, "opacity's default is its maximum");
        for _ in 0..3 {
            let task = update(&mut g, Message::KeyPressed(arrow_right()));
            assert_eq!(g.config.opacity, 100, "a held Right stops at the bound");
            assert_eq!(task.units(), 0, "and puts no command on the socket");
        }
        // And the other end, reached by stepping down to it.
        for _ in 0..101 {
            let _ = update(&mut g, Message::KeyPressed(arrow_down()));
        }
        assert_eq!(g.config.opacity, 0, "a hundred steps reach the minimum");
        let task = update(&mut g, Message::KeyPressed(arrow_down()));
        assert_eq!(g.config.opacity, 0, "and one more stays there");
        assert_eq!(task.units(), 0, "with nothing sent");
    }

    /// Enter commits what is typed and hands typing back to the row. The typed
    /// value is applied by `Message::NumText` the moment it is valid —
    /// that is the row's existing behaviour and it is why a draft can only ever
    /// hold refused text — so what the commit has left to do is refuse it
    /// properly or hand the input back.
    #[test]
    fn enter_inside_a_number_row_commits_and_hands_typing_back() {
        let mut g = gui();
        g.focus = Some(FocusTarget::Field(Key::Spacing));
        let _ = update(&mut g, Message::NumText(Key::Spacing, "9".into()));
        assert_eq!(g.config.spacing, 9, "a valid value applies as it is typed");

        let task = update(&mut g, Message::NumSubmit(Key::Spacing));
        assert_eq!(g.config.spacing, 9, "nothing is left to apply");
        assert_eq!(g.focus, Some(FocusTarget::Field(Key::Spacing)));
        assert_eq!(
            task.units(),
            1,
            "the commit hands typing back, which is the one operation it runs"
        );
        assert!(
            g.num_drafts.is_empty(),
            "and leaves no half-typed text behind"
        );
    }

    /// A refused commit answers with the daemon's own error text, changes no
    /// value, and stays in the input: `None` is what tells the caller the caret
    /// is still wanted there. The wording is the parser's, not a second one.
    #[test]
    fn a_refused_commit_answers_the_error_and_keeps_the_caret() {
        let mut g = gui();
        g.focus = Some(FocusTarget::Field(Key::Spacing));
        let _ = update(&mut g, Message::NumText(Key::Spacing, "99".into()));
        assert_eq!(
            g.config.spacing, 4,
            "99 is out of range, so it stays a draft"
        );

        assert!(
            commit_num(&mut g, Key::Spacing).is_none(),
            "a value outside the bounds is refused"
        );
        assert_eq!(g.config.spacing, 4, "and the row keeps the value it had");
        assert_eq!(g.focus, Some(FocusTarget::Field(Key::Spacing)));
        assert_eq!(
            g.last_reply.text(),
            "error: spacing <0-24>",
            "the refusal is the daemon's own wording for this key"
        );
        assert_eq!(
            g.num_drafts.get(&Key::Spacing).map(String::as_str),
            Some("99"),
            "and the refused text stays for the user to fix"
        );
    }

    /// Tab out of a number row commits what is typed there instead of moving
    /// on, and a commit the bounds refuse keeps the caret in the input — so it
    /// keeps the ring too. Nothing pending means an ordinary Tab stop: the
    /// only text a row can hold is text `NumText` refused, because a value the
    /// bounds allow is applied the moment it is typed.
    #[test]
    fn tab_out_of_a_number_row_commits_instead_of_moving_on() {
        let mut g = gui();
        g.focus = Some(FocusTarget::Field(Key::Spacing));
        let _ = update(&mut g, Message::NumText(Key::Spacing, "99".into()));

        let refused = update(&mut g, Message::KeyPressed(no_key(tab_key())));
        assert_eq!(g.focus, Some(FocusTarget::Field(Key::Spacing)));
        assert_eq!(
            refused.units(),
            0,
            "a refused commit runs nothing at all, so nothing can move"
        );
        assert_eq!(
            g.last_reply.text(),
            "error: spacing <0-24>",
            "and says why, the way the commit does"
        );

        // Fixing the text applies it, which clears what Tab was about to
        // commit, so the next Tab is a plain move — and it takes typing with it.
        let _ = update(&mut g, Message::NumText(Key::Spacing, "8".into()));
        assert_eq!(g.config.spacing, 8);
        let moved = update(&mut g, Message::KeyPressed(no_key(tab_key())));
        assert_eq!(field(&g), Key::MaxName, "Tab moved to the next row");
        assert_eq!(
            moved.units(),
            2,
            "the reveal of the row it landed on, plus the hand-back that stops \
             the input it left holding the keyboard"
        );
    }

    /// Escape that the input swallowed drops the row's half-typed value and
    /// hands typing back, applying nothing — the row shows the value it
    /// had before the typing began. Iced's own Escape arm does the unfocusing
    /// and tells nobody, which is why the app is told separately.
    #[test]
    fn escape_drops_the_typed_value_without_applying_it() {
        let mut g = gui();
        g.focus = Some(FocusTarget::Field(Key::Spacing));
        let _ = update(&mut g, Message::NumText(Key::Spacing, "99".into()));
        assert_eq!(g.config.spacing, 4);

        let task = update(&mut g, Message::EscapeCaptured);
        assert_eq!(g.config.spacing, 4, "nothing was applied");
        assert_eq!(g.focus, Some(FocusTarget::Field(Key::Spacing)));
        assert!(
            g.num_drafts.is_empty(),
            "the row goes back to showing its own value"
        );
        assert_eq!(task.units(), 1, "and typing is handed back to the row");

        // Which is what lets the next Tab move on rather than commit again.
        let _ = update(&mut g, Message::KeyPressed(no_key(tab_key())));
        assert_eq!(field(&g), Key::MaxName, "the row is an ordinary stop again");
    }

    /// Tab far enough down to the page's last *keyed* row, then run the reveal
    /// that row needs: it sits in the final screenful, so the reveal target is
    /// past the end of the page, which is where a scrollable clamps it. The
    /// sidebar must still name the row's own section — the ring is on it, and
    /// Ctrl+R resets whatever the sidebar names. The trap is Connection: a page
    /// read as "scrolled to its end" names it, and it is the one section with no
    /// config group, so Ctrl+R then resets nothing at all.
    #[test]
    fn the_sidebar_names_the_section_of_the_focused_row_at_the_page_end() {
        let mut g = gui();
        // Three header actions and five sidebar buttons come first, so Tab once
        // per keyed row after them lands on the last one. The credential rows
        // render after every keyed row, so counting them in here would land on
        // a credential instead and lose the row this test is about.
        let keyed = fields::rendered_targets("")
            .filter(|t| matches!(t, FocusTarget::Field(_)))
            .count();
        for _ in 0..(3 + Section::ALL.len() + keyed) {
            let _ = update(
                &mut g,
                Message::KeyPressed(no_key(keyboard::Key::Named(keyboard::key::Named::Tab))),
            );
        }
        assert_eq!(field(&g), Key::BoxColor, "the last keyed row on the page");

        // The reveal that row needs: its top minus the margin, which is past
        // the end of the page. A `Task` is inert in a unit test, so this is the
        // request the reveal issues and the state it leaves behind, never the
        // position a scrollable would settle on. It must move the page and
        // nothing else.
        let _ = update(&mut g, Message::ScrollContentTo(2100.0));
        assert_eq!(
            g.section,
            Section::Colors,
            "the ring is on a Colors row, so the sidebar has to name Colors"
        );
    }

    /// The two credential rows are the last rows the page renders, so from a
    /// known start they are a known number of Tabs away: three header actions,
    /// five sidebar items, and one press per row before them. Tabbed *to* them
    /// in both directions, because Tab reaching a row only in one direction
    /// means a Shift+Tab user cannot reach it at all.
    #[test]
    fn tab_reaches_the_credential_rows_from_the_top_and_from_the_bottom() {
        let mut g = gui();
        // Forward from nothing: three header actions, five sidebar items, one
        // press per keyed row, then the credential.
        let to_first = 3 + Section::ALL.len() + keyed_rows() + 1;
        tab_n(&mut g, to_first);
        assert_eq!(g.focus, Some(FocusTarget::Credential(Credential::ClientId)));

        tab_n(&mut g, 1);
        assert_eq!(
            g.focus,
            Some(FocusTarget::Credential(Credential::ClientSecret))
        );

        // Shift+Tab back out of the pair lands on the last keyed row, which is
        // what the pair was appended after: adding credentials renumbered no
        // Tab above them.
        shift_tab_n(&mut g, 2);
        assert_eq!(
            g.focus,
            Some(FocusTarget::Field(Key::BoxColor)),
            "the row before the first credential is still the last keyed row"
        );
    }

    /// Both ends of the order wrap onto the credential rows the same way every
    /// other target does: Shift+Tab off the daemon toggle (the last) reaches
    /// the last credential by wrapping past the header, and Tab forward off
    /// the end reaches the first credential. Getting this wrong strands a
    /// keyboard user on a row they cannot leave in one direction.
    #[test]
    fn tab_wraps_onto_and_off_the_credential_rows() {
        let mut g = gui();
        g.focus = Some(FocusTarget::ToggleDaemon);
        shift_tab_n(&mut g, 1);
        assert_eq!(
            g.focus,
            Some(FocusTarget::Credential(Credential::ClientSecret)),
            "Shift+Tab off the last target wraps to the last credential"
        );

        shift_tab_n(&mut g, 1);
        assert_eq!(
            g.focus,
            Some(FocusTarget::Credential(Credential::ClientId)),
            "and Shift+Tab again reaches the first credential"
        );

        // Tab off the last target still wraps to the first, so the pair is
        // inside the same cycle rather than appended after it.
        g.focus = Some(FocusTarget::ToggleDaemon);
        tab_n(&mut g, 1);
        assert_eq!(g.focus, Some(FocusTarget::ClearChanges));
        tab_n(&mut g, 3 + Section::ALL.len() + keyed_rows());
        assert_eq!(
            g.focus,
            Some(FocusTarget::Credential(Credential::ClientId)),
            "Tab forward through the chrome reaches the first credential"
        );
        tab_n(&mut g, 1);
        assert_eq!(
            g.focus,
            Some(FocusTarget::Credential(Credential::ClientSecret))
        );
        tab_n(&mut g, 1);
        assert_eq!(
            g.focus,
            Some(FocusTarget::ToggleDaemon),
            "and Tab on leaves the credentials for the toggle, as for every row"
        );
    }

    /// Focus on a credential row hands typing to that row's text input, and
    /// Tab off it hands typing back — iced's focus is what makes typing work at
    /// all, and it is not the same thing as the ring. Left holding the input,
    /// a Tab away from a credential would keep sending every later keystroke
    /// into it, silently rewriting a secret the user is no longer looking at.
    /// Read off the returned `Task`: the number of operations it runs is the
    /// only thing a unit test can see, and a credential must run two (focus
    /// the input, measure the reveal) where a keyed row ran one.
    #[test]
    fn focus_hands_typing_to_a_credential_row_and_takes_it_back_on_tab_away() {
        let mut g = gui();
        g.focus = Some(FocusTarget::Credential(Credential::ClientId));
        let onto = update(&mut g, Message::KeyPressed(no_key(tab_key())));
        assert_eq!(
            onto.units(),
            2,
            "landing on a credential runs the focus hand-off and the reveal"
        );

        // And off it again, to a target that is not a credential.
        let off = update(&mut g, Message::KeyPressed(no_key(tab_key())));
        assert_eq!(
            off.units(),
            1,
            "Tab off a credential runs the hand-back and nothing else: no row, \
             so no reveal"
        );
    }

    /// Enter on a focused credential does nothing: the keystrokes that built
    /// the draft were already the input's, and committing the pair is the
    /// "apply connection" button's job. Pinned because a credential that grew
    /// an Enter arm would commit a half-typed secret with no confirmation.
    #[test]
    fn enter_on_a_credential_row_is_a_noop() {
        let mut g = gui();
        g.focus = Some(FocusTarget::Credential(Credential::ClientId));
        let _ = update(&mut g, Message::KeyPressed(no_key(enter_key())));
        assert_eq!(
            g.auth_client_id, "",
            "no apply, and no daemon restart: that is the button's job"
        );
    }

    /// A narrowed search drops the rows it does not render, so Tab walks only
    /// the hits.
    #[test]
    fn a_search_narrows_the_tab_order_to_the_rows_it_renders() {
        let mut g = gui();
        g.search = "color".to_string();
        let order = tab_order(&g);
        let fields: Vec<Key> = order
            .iter()
            .filter_map(|t| match t {
                FocusTarget::Field(key) => Some(*key),
                _ => None,
            })
            .collect();
        assert_eq!(
            fields,
            [Key::SpeakingColor, Key::TextColor, Key::BoxColor],
            "only the three hits the search page renders are in the order"
        );
    }

    /// Tab on the search page reaches the hits: the chrome and the sidebar come
    /// first, so the first field is the next press after them.
    #[test]
    fn tab_on_the_search_page_walks_the_hits() {
        let mut g = gui();
        g.search = "color".to_string();
        for _ in 0..(4 + Section::ALL.len()) {
            let _ = update(
                &mut g,
                Message::KeyPressed(no_key(keyboard::Key::Named(keyboard::key::Named::Tab))),
            );
        }
        assert_eq!(
            field(&g),
            Key::SpeakingColor,
            "the press after the chrome lands on the first hit"
        );
    }

    /// Clearing the search lands the one-pager back on the offset tracked
    /// before it, but nothing re-derived the highlight for that offset:
    /// `move_focus` had left `gui.section` naming the search hit's section,
    /// so the sidebar named a row the restored viewport does not show.
    /// Restoring therefore has to measure as well as scroll. A `Task` counts
    /// the operations it runs in `units()`, which is the only thing a unit
    /// test can read off it — the highlighted section itself only changes
    /// when the pass this guard asks for reports back below.
    #[test]
    fn clearing_the_search_re_derives_the_highlight_at_the_restored_offset() {
        let mut g = gui();
        // Where the user left the one-pager: a screenful into Opacity.
        let _ = update(&mut g, Message::Scrolled(1900.0));
        let _ = update(
            &mut g,
            Message::Measured {
                offsets: one_pager_headers(),
                max_scroll: 2600.0,
                jump: None,
            },
        );
        assert_eq!(g.section, Section::Opacity);

        // The search page renders no sections, but the ring is on a hit and
        // Ctrl+R has to reach that hit's section while the search is up.
        g.search = "color".to_string();
        for _ in 0..(4 + Section::ALL.len()) {
            let _ = update(
                &mut g,
                Message::KeyPressed(no_key(keyboard::Key::Named(keyboard::key::Named::Tab))),
            );
        }
        let _ = update(
            &mut g,
            Message::KeyPressed(no_key(keyboard::Key::Named(keyboard::key::Named::Tab))),
        );
        assert_eq!(field(&g), Key::TextColor);
        assert_eq!(g.section, Section::Colors, "the ring is on a Colors row");

        // Esc empties the search, so the one-pager returns and the sidebar
        // must name the section of the offset it returns to, not the hit.
        let task = update(
            &mut g,
            Message::KeyPressed(no_key(keyboard::Key::Named(keyboard::key::Named::Escape))),
        );
        assert_eq!(
            task.units(),
            2,
            "the restore has to run the measure pass that re-derives the \
             highlight, not only the scroll"
        );
        // The typed clear takes the same path, so it must measure too.
        g.search = "color".to_string();
        let task = update(&mut g, Message::Search(String::new()));
        assert_eq!(
            task.units(),
            2,
            "same restore, reached by emptying the input"
        );

        // What that pass reports: Opacity, the section the restored offset
        // is in, rather than the Colors the search page left behind.
        let _ = update(
            &mut g,
            Message::Measured {
                offsets: one_pager_headers(),
                max_scroll: 2600.0,
                jump: None,
            },
        );
        assert_eq!(g.section, Section::Opacity);
    }

    /// Section header offsets of a one-pager tall enough to scroll, in
    /// `Section::ALL` order. Same numbers the scrollspy's own tests use.
    fn one_pager_headers() -> [f32; Section::ALL.len()] {
        [8.0, 900.0, 1500.0, 2100.0, 2600.0]
    }

    /// Narrowing the search can drop the focused row: the ring is drawn only
    /// where the page renders a row carrying that key, so keeping the focus
    /// would leave nothing ringed anywhere while Enter still fired the command
    /// for a row that is not on screen.
    #[test]
    fn narrowing_the_search_drops_a_focus_it_no_longer_renders() {
        let mut g = gui();
        g.focus = Some(FocusTarget::Field(Key::Rtl));
        let _ = update(&mut g, Message::Search("color".to_string()));
        assert_eq!(
            g.focus, None,
            "`rtl` is not one of the rows the search page renders, so it cannot stay focused"
        );
    }

    /// The focused field's key, or a panic naming what focus is on instead.
    fn field(g: &Gui) -> Key {
        match g.focus {
            Some(FocusTarget::Field(key)) => key,
            other => panic!("expected a focused field, got {other:?}"),
        }
    }

    /// Enter with nothing focused is a no-op.
    #[test]
    fn enter_with_no_focus_is_a_noop() {
        let mut g = gui();
        let _ = update(
            &mut g,
            Message::KeyPressed(no_key(keyboard::Key::Named(keyboard::key::Named::Enter))),
        );
        assert_eq!(g.focus, None);
    }

    fn key_event(key: keyboard::Key, modifiers: keyboard::Modifiers) -> keyboard::Event {
        keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers,
            repeat: false,
            text: None,
        }
    }

    fn no_key(key: keyboard::Key) -> keyboard::Event {
        key_event(key, keyboard::Modifiers::default())
    }

    fn arrow_left() -> keyboard::Event {
        no_key(keyboard::Key::Named(key::Named::ArrowLeft))
    }

    fn arrow_right() -> keyboard::Event {
        no_key(keyboard::Key::Named(key::Named::ArrowRight))
    }

    fn arrow_up() -> keyboard::Event {
        no_key(keyboard::Key::Named(key::Named::ArrowUp))
    }

    fn arrow_down() -> keyboard::Event {
        no_key(keyboard::Key::Named(key::Named::ArrowDown))
    }

    /// The same key as [`arrow_right`], arriving the way a held one does: X11
    /// auto-repeat is a stream of further KeyPressed events, each one a step.
    fn held_right() -> keyboard::Event {
        match arrow_right() {
            keyboard::Event::KeyPressed {
                key,
                modified_key,
                physical_key,
                location,
                modifiers,
                text,
                ..
            } => keyboard::Event::KeyPressed {
                key,
                modified_key,
                physical_key,
                location,
                modifiers,
                repeat: true,
                text,
            },
            other => other,
        }
    }

    fn shift_tab() -> keyboard::Event {
        key_event(
            keyboard::Key::Named(keyboard::key::Named::Tab),
            keyboard::Modifiers::SHIFT,
        )
    }

    fn tab_key() -> keyboard::Key {
        keyboard::Key::Named(keyboard::key::Named::Tab)
    }

    fn enter_key() -> keyboard::Key {
        keyboard::Key::Named(keyboard::key::Named::Enter)
    }

    /// The number of keyed rows on the one-pager, which is what the tab
    /// arithmetic counts through: the credential rows come after all of them,
    /// so a test that wants a credential counts these.
    fn keyed_rows() -> usize {
        fields::rendered_targets("")
            .filter(|t| matches!(t, FocusTarget::Field(_)))
            .count()
    }

    fn tab_n(gui: &mut Gui, times: usize) {
        for _ in 0..times {
            let _ = update(gui, Message::KeyPressed(no_key(tab_key())));
        }
    }

    fn shift_tab_n(gui: &mut Gui, times: usize) {
        for _ in 0..times {
            let _ = update(gui, Message::KeyPressed(shift_tab()));
        }
    }
}
