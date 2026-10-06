//! View layer: the window composition — header (title, search, global
//! actions), sidebar (section anchors + shortcut cheat-sheet), status bar
//! (unsaved marker, daemon toggle, last reply) — around the section pages
//! that `fields` renders.

use hyprlay_core::domain::Reply;
use hyprlay_core::status::StatusFields;
use iced::Alignment;
use iced::Element;
use iced::Length;
use iced::widget::button;
use iced::widget::column;
use iced::widget::container;
use iced::widget::row;
use iced::widget::text;
use iced::widget::text_input;

use super::FocusTarget;
use super::Gui;
use super::Message;
use super::fields::Section;
use super::fields::search_page;
use super::fields::settings_page;
use super::scroll::widget_id;
use super::theme::AMBER;
use super::theme::BRIGHT;
use super::theme::DANGER;
use super::theme::HEADER_BG;
use super::theme::MUTED;
use super::theme::REPLY_GREEN;
use super::theme::SIDEBAR_BG;
use super::theme::nav_style;
use super::theme::panel;
use super::theme::plain_style;
use super::theme::primary_style;

pub(super) fn view(gui: &Gui) -> Element<'_, Message> {
    let content = if gui.search.trim().is_empty() {
        settings_page(gui)
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
    let focused = |t: FocusTarget| gui.focus == Some(t);
    // "Clear changes" only does something while the runtime config differs
    // from disk; a disabled press target communicates that at a glance. It
    // still takes focus, and reads as inert when disabled.
    let mut clear =
        button(text("Clear changes")).style(plain_style(focused(FocusTarget::ClearChanges)));
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
                .style(plain_style(focused(FocusTarget::ResetAll))),
            button(text("Save"))
                .on_press(Message::Save)
                .style(primary_style(gui.dirty, focused(FocusTarget::Save))),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding([8, 12])
    .width(Length::Fill)
    .style(panel(HEADER_BG))
    .into()
}

/// The shortcut cheat-sheet, in the bottom-left of the sidebar: every binding
/// the window honours, read off the dispatcher in `update::shortcut` and the
/// captured-Escape arm of `subscribe`.
///
/// Two columns — the keys, then what the key does — and the layout rules are
/// not taste:
///
/// - The key column is 12 wide, `Ctrl+Shift+R` being the longest key bound.
/// - No line is wider than the sidebar, which
///   `the_sheet_fits_the_sidebar` holds every line to. A longer line runs past
///   its own panel and over the content behind it.
/// - Every word in the right column is an outcome `sheet_tests` recognises, so
///   a line cannot claim a key does something it does not.
///
/// Three checks keep it true, and none of them compares the list with itself.
/// `every_binding_the_window_honours_is_on_the_sheet` drives the dispatcher
/// with every key and compares what the window did against what the sheet
/// says, in both directions: a binding added anywhere in `update.rs` fails for
/// having no line, and a line naming a binding the app does not have fails the
/// other way. `the_sheet_fits_the_sidebar` holds every line inside the panel.
/// `an_arrow_steps_the_way_its_line_says` reads the arrow direction off the
/// renderer's own option table, which the state diff alone cannot show.
const SHORTCUTS: &str = "\
shortcuts
Tab/Shift+Tab  focus/apply
Enter/Space  flip/step/type
Enter/Space  save/all/jump
Left/Down  step back
Right/Up  step forward
R  reset/unsaved
Esc  leave
Esc  clear
Ctrl+S  save
Ctrl+R  section
Ctrl+Shift+R  all
Ctrl+F  search
Ctrl+1..5  jump/clear
";

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
            .on_press(Message::Navigate(*s))
            .width(Length::Fill)
            .style(nav_style(selected, gui.focus == Some(FocusTarget::Nav(i)))),
        );
    }
    let col = column![
        nav,
        iced::widget::Space::new().height(Length::Fill),
        text(SHORTCUTS).size(10).color(MUTED),
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
            text(match gui.last_reply {
                Reply::Error(_) => "error".to_string(),
                _ => gui.last_reply.text().to_string(),
            })
            .size(11)
            .color(match gui.last_reply {
                Reply::Error(_) => DANGER,
                _ => REPLY_GREEN,
            }),
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
    let mut toggle = button(text(gui.daemon_state.label()).size(11))
        .style(plain_style(gui.focus == Some(FocusTarget::ToggleDaemon)));
    if gui.daemon_state.toggle().is_some() {
        toggle = toggle.on_press(Message::ToggleDaemon);
    }
    toggle.into()
}

/// "status=connected channel=ngobrol 3 participants=2 …" →
/// "connected · ngobrol 3". Parsing goes through the shared
/// [`StatusFields`]; channel names may contain spaces, which its
/// marker-slice handles.
fn brief_status(full: &str) -> String {
    match StatusFields::parse_wire(full) {
        Some(fields) if !fields.channel.is_empty() => {
            format!("{} · {}", fields.status_word, fields.channel)
        }
        Some(fields) => fields.status_word.to_string(),
        None => full.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brief_status_keeps_multiword_channel_names_intact() {
        let full = "status=connected channel=ngobrol 3 participants=2 rtl=on monitor=eDP-1";
        assert_eq!(brief_status(full), "connected · ngobrol 3");
    }

    #[test]
    fn brief_status_without_channel_falls_back_to_connection_word() {
        assert_eq!(brief_status("status=disconnected"), "disconnected");
        assert_eq!(brief_status("connecting…"), "connecting…");
        // Empty channel value (malformed Discord payload only): the old
        // code printed "connected · " with a trailing separator; the new
        // code drops it. Pinned here so the tightening stays deliberate.
        assert_eq!(
            brief_status("status=connected channel= participants=0"),
            "connected"
        );
    }
}

/// The cheat-sheet checked against the window, so the list cannot go stale.
///
/// Two sources and no third: [`SHORTCUTS`] is the prose, and the truth is what
/// the real dispatcher does to a [`Gui`] when driven with each keystroke. A new
/// binding in `update.rs` shows up here as a key the window honours, which no
/// line names, and the check fails. A line naming a binding the window does not
/// honour fails the other way. Neither direction compares the sheet with itself,
/// so neither can pass by the list agreeing with itself.
#[cfg(test)]
mod sheet_tests {
    use std::collections::BTreeSet;

    use hyprlay_core::config::AnchorMode;
    use hyprlay_core::domain::Key;
    use hyprlay_core::domain::Value;
    use iced::keyboard::key;
    use iced::keyboard::{self};

    use super::SHORTCUTS;
    use crate::gui::FocusTarget;
    use crate::gui::Gui;
    use crate::gui::Message;
    use crate::gui::fields::Section;
    use crate::gui::fields::options;
    use crate::gui::test_gui;
    use crate::gui::update::update;

    /// What one keystroke did to one window, as the sheet is allowed to
    /// describe it. Every variant is something the window state shows: what
    /// the ring is on, what the focused row holds, whether the search is up,
    /// and the unsaved marker. A word the sheet spends maps to one of these, so
    /// a line claiming an effect the key does not have cannot pass.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
    enum Effect {
        /// The ring moved to another target.
        Focus,
        /// The focused row's value moved: a flag flipped, an option stepped, a
        /// number stepped, or R put one back to its default.
        Value,
        /// The search emptied.
        Clear,
        /// The page moved to a section — Ctrl+1..5, which scrolls the one-pager
        /// to that section's header.
        Jump,
        /// The unsaved marker cleared — Save.
        Save,
        /// The unsaved marker rose — one of the three resets, which all go out
        /// as a command and wait for the daemon's `dump`.
        Reset,
    }

    /// What one word of the sheet claims. Every word is a change the window
    /// state shows, except the hand-off.
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    enum Claim {
        /// The changes to the window state this word claims, compared against
        /// what the sweep saw.
        Effects(BTreeSet<Effect>),
        /// A widget hand-off, invisible in `Gui` and checked elsewhere: into a
        /// row's number input, into the search box, and back out again.
        HandedOver,
    }

    /// The closed vocabulary the sheet may spend. A word outside it fails, so
    /// the list cannot drift into prose this check cannot judge.
    ///
    /// A word may claim more than one effect, and one of them has to: every
    /// setting-changing key also marks the window unsaved on the states where
    /// auto-save is off, so a word that named only the value move would fail on
    /// those states. Keeping that coupling in the table rather than in the sheet
    /// is what lets the words stay short enough to fit the panel.
    fn claimed(word: &str) -> Option<Claim> {
        let effects: &[Effect] = match word {
            "focus" => &[Effect::Focus],
            // A value that moved off the saved one raises the marker, so these
            // claim it too. `back` and `forward` are the direction a step went,
            // which the window state cannot show on its own —
            // `an_arrow_steps_the_way_its_line_says` reads that off the
            // renderer's option table instead. `apply` is a value moving under
            // Tab, which commits a number row's draft.
            "flip" | "step" | "back" | "forward" | "apply" | "reset" => {
                &[Effect::Value, Effect::Reset]
            }
            // A reset that moves no value — the section and global ones, which
            // wait for the daemon's `dump` — leaves only the marker.
            "section" | "all" | "unsaved" => &[Effect::Reset],
            "clear" => &[Effect::Clear],
            "jump" => &[Effect::Jump],
            "save" => &[Effect::Save],
            // Hand-offs: into a row's number input, into the search box, and back
            // out again. No trace in `Gui`; each is pinned by its own test.
            "type" | "search" | "leave" => &[],
            _ => return None,
        };
        // An empty claim is the hand-off, which is what the sweep cannot see.
        Some(if effects.is_empty() {
            Claim::HandedOver
        } else {
            Claim::Effects(effects.iter().copied().collect())
        })
    }

    /// Every keystroke the sweep presses: each letter and digit, and each named
    /// key, in all four plain/Shift/Ctrl/Ctrl+Shift combinations. The dismissal
    /// is drawn from `binding_name`, not this list: a sweep that only pressed
    /// the combinations the sheet spells could not tell a binding that reads a
    /// modifier from one that ignores it.
    fn keys() -> Vec<(String, Message)> {
        // All four modifier combinations are pressed for every key, not just the
        // ones the sheet happens to spell; `binding_name` is where the
        // sheet-named ones are told apart from the superfluous-modifier forms.
        let modifiers: [(&str, keyboard::Modifiers); 4] = [
            ("", keyboard::Modifiers::default()),
            ("Shift+", keyboard::Modifiers::SHIFT),
            ("Ctrl+", keyboard::Modifiers::CTRL),
            (
                "Ctrl+Shift+",
                keyboard::Modifiers::CTRL | keyboard::Modifiers::SHIFT,
            ),
        ];
        let mut out = Vec::new();
        let press_all = |out: &mut Vec<(String, Message)>, spelled: &str, key: keyboard::Key| {
            for (prefix, modifiers) in modifiers.iter().copied() {
                out.push((format!("{prefix}{spelled}"), press(key.clone(), modifiers)));
            }
        };
        for c in 'a'..='z' {
            // Spelled the way the sheet spells a key: capitals, since the
            // dispatcher matches the letter case-blind.
            press_all(
                &mut out,
                &c.to_ascii_uppercase().to_string(),
                keyboard::Key::Character(c.to_string().into()),
            );
        }
        for c in '0'..='9' {
            press_all(
                &mut out,
                &c.to_string(),
                keyboard::Key::Character(c.to_string().into()),
            );
        }
        for (named, spelled) in [
            (key::Named::Tab, "Tab"),
            (key::Named::Enter, "Enter"),
            (key::Named::Space, "Space"),
            (key::Named::Escape, "Esc"),
            (key::Named::ArrowLeft, "Left"),
            (key::Named::ArrowDown, "Down"),
            (key::Named::ArrowRight, "Right"),
            (key::Named::ArrowUp, "Up"),
            (key::Named::Home, "Home"),
            (key::Named::End, "End"),
            (key::Named::PageUp, "PageUp"),
            (key::Named::PageDown, "PageDown"),
            (key::Named::Delete, "Delete"),
            (key::Named::Insert, "Insert"),
            (key::Named::Backspace, "Backspace"),
        ] {
            press_all(&mut out, spelled, keyboard::Key::Named(named));
        }
        out
    }

    fn press(key: keyboard::Key, modifiers: keyboard::Modifiers) -> Message {
        Message::KeyPressed(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers,
            repeat: false,
            text: None,
        })
    }

    /// One window per state a binding needs to show itself in. A closure each
    /// rather than a list of values: `Gui` is not `Clone`, and every keystroke
    /// gets a window of its own.
    ///
    /// The states are chosen so that every arm of the dispatcher is reachable
    /// from at least one of them. A binding whose effect depends on state the
    /// sweep never visits is a binding the sweep cannot see, so a new one would
    /// pass the check silently — which is the failure this check exists to
    /// prevent. That means the chrome controls and the credential rows get
    /// their own windows, not just the field rows, because Enter and Tab reach
    /// those too.
    fn states() -> Vec<fn() -> Gui> {
        vec![
            // A search up, so Escape has something to clear. Deliberately on
            // the first section: a Ctrl+1..5 taken here would clear the search
            // as well as jumping (D3), and those two are one key doing two
            // things, which the sheet does not need to spell out.
            || test_gui("avatar"),
            // Standing on a section that is not the first, so every Ctrl+1..5
            // moves the page somewhere else rather than onto the section it
            // already stands on — which is what makes the jump observable.
            || {
                let mut gui = test_gui("");
                gui.section = Section::Colors;
                gui
            },
            // A clean window with nothing focused: the Ctrl keys work whatever
            // the ring is on.
            || test_gui(""),
            // Unsaved, for Save to clear the marker.
            || {
                let mut gui = test_gui("");
                gui.dirty = true;
                gui
            },
            // Clean with auto-save off, which is the only way a reset shows:
            // all three go out as a command and wait for the daemon's `dump`,
            // and with auto-save on they leave nothing behind to see.
            || {
                let mut gui = test_gui("");
                gui.config.auto_save = false;
                gui
            },
            // …and with a setting off its default as well, so the per-field
            // reset raises the marker too. Without this, `R` would only ever
            // be seen moving a value and a reset that forgot to mark itself
            // unsaved would pass.
            || {
                let mut gui = test_gui("");
                gui.config.auto_save = false;
                gui.config.spacing = 8;
                gui.focus = Some(FocusTarget::Field(Key::Spacing));
                gui
            },
            // A flag off its default, so Enter flips it and R puts it back.
            || {
                let mut gui = test_gui("");
                gui.config.rtl = true;
                gui.focus = Some(FocusTarget::Field(Key::Rtl));
                gui
            },
            // An option select on its first choice, so a forward arrow has one
            // to step to and a backward one is on the boundary.
            || {
                let mut gui = test_gui("");
                gui.focus = Some(FocusTarget::Field(Key::Anchor));
                gui
            },
            // …and on its last, for the other direction.
            || {
                let mut gui = test_gui("");
                gui.config.anchor = AnchorMode::Bottom;
                gui.focus = Some(FocusTarget::Field(Key::Anchor));
                gui
            },
            // A number off its default, so the arrows have room in both
            // directions and R has something to restore.
            || {
                let mut gui = test_gui("");
                gui.config.spacing = 8;
                gui.focus = Some(FocusTarget::Field(Key::Spacing));
                gui
            },
            // A number row holding a draft the bounds refuse. Tab commits what
            // is held, Escape comes back out of it, and neither path is
            // reachable without a draft in hand — so a binding that only fires
            // on one of them would otherwise go unseen.
            || {
                let mut gui = test_gui("");
                gui.config.spacing = 8;
                gui.focus = Some(FocusTarget::Field(Key::Spacing));
                gui.num_drafts.insert(Key::Spacing, "99".into());
                gui
            },
            // …and the same row holding a draft the bounds allow, which is the
            // path where committing actually moves the value.
            || {
                let mut gui = test_gui("");
                gui.config.spacing = 4;
                gui.focus = Some(FocusTarget::Field(Key::Spacing));
                gui.num_drafts.insert(Key::Spacing, "9".into());
                gui
            },
            // The ring on Save, so Enter and Space reach the save arm the way
            // they reach a row's.
            || {
                let mut gui = test_gui("");
                gui.dirty = true;
                gui.focus = Some(FocusTarget::Save);
                gui
            },
            // The ring on Reset all, so Enter reaches that arm too. Auto-save off is what
            // makes the marker rise, which is the only trace a global reset
            // leaves in the window.
            || {
                let mut gui = test_gui("");
                gui.config.auto_save = false;
                gui.focus = Some(FocusTarget::ResetAll);
                gui
            },
            // The ring on a sidebar item, so Enter reaches the navigate arm: a
            // different effect from any field row, on the same key. Focused on
            // a different section from the one shown, so the jump is visible.
            || {
                let mut gui = test_gui("");
                gui.section = Section::Colors;
                gui.focus = Some(FocusTarget::Nav(1));
                gui
            },
            // The ring on a credential. A credential edits no config key, so it
            // has no value to show, and Enter on one is a no-op by design — but
            // Tab onto one hands the keyboard over, which is a real effect.
            || {
                let mut gui = test_gui("");
                gui.focus = Some(FocusTarget::Credential(crate::gui::Credential::ClientId));
                gui
            },
        ]
    }

    /// The window as the outside sees it, reduced to what the sheet can claim.
    #[derive(PartialEq)]
    struct Seen {
        focus: Option<FocusTarget>,
        search: String,
        section: crate::gui::fields::Section,
        dirty: bool,
        value: Option<hyprlay_core::domain::Value>,
    }

    fn seen(gui: &Gui) -> Seen {
        Seen {
            focus: gui.focus,
            search: gui.search.clone(),
            section: gui.section,
            dirty: gui.dirty,
            value: match gui.focus {
                Some(FocusTarget::Field(key)) => Some(key.value_of(&gui.config)),
                _ => None,
            },
        }
    }

    /// What one keystroke did to one window: the changes it made, and whether
    /// it was honoured at all. An unbound key reaches no arm and returns
    /// `Task::none()` with nothing changed; a bound one does at least one of
    /// the two. Both halves are needed, because two of the bindings here are
    /// invisible in the state — `Ctrl+F` and Enter on a number row hand the
    /// keyboard to a widget and change nothing this struct can see — and
    /// calling those unbound would be the same error as listing a key the app
    /// does not have.
    fn effects(message: Message, make: fn() -> Gui) -> (BTreeSet<Effect>, bool) {
        let mut gui = make();
        let was = seen(&gui);
        let spoke = update(&mut gui, message).units() > 0;
        let now = seen(&gui);

        let mut out = BTreeSet::new();
        if now.focus != was.focus {
            out.insert(Effect::Focus);
        }
        if !was.search.is_empty() && now.search.is_empty() {
            out.insert(Effect::Clear);
        }
        // A section change counts as a jump only on its own: Tab onto a field row
        // repoints the sidebar at that row's section so Ctrl+R resets the
        // section the user is looking at (see `move_focus`), which is the
        // reveal's bookkeeping rather than something the key did for you.
        if now.focus == was.focus && now.section != was.section {
            out.insert(Effect::Jump);
        }
        if was.dirty && !now.dirty {
            out.insert(Effect::Save);
        }
        if !was.dirty && now.dirty {
            out.insert(Effect::Reset);
        }
        if now.focus == was.focus && now.value != was.value {
            out.insert(Effect::Value);
        }
        let honoured = spoke || !out.is_empty();
        (out, honoured)
    }

    /// Every binding the window honours: each key the dispatcher acts on, with
    /// the effects it was seen to have across the states above. Two probes
    /// under one name where the dispatcher has two arms for the key — Escape
    /// clears the search itself and also arrives captured, out of an input,
    /// which `subscribe` forwards as its own message.
    fn bindings() -> Vec<(String, BTreeSet<Effect>)> {
        let mut out: Vec<(String, BTreeSet<Effect>)> = Vec::new();
        for (spelled, message) in keys() {
            let mut all = BTreeSet::new();
            let mut honoured = false;
            for make in states() {
                let (found, yes) = effects(message.clone(), make);
                all.extend(found);
                honoured |= yes;
            }
            if honoured {
                let name = binding_name(&spelled);
                match out.iter_mut().find(|(key, _)| *key == name) {
                    Some((_, seen)) => seen.extend(all),
                    None => out.push((name, all)),
                }
            }
        }
        let mut captured = BTreeSet::new();
        let mut honoured = false;
        for make in states() {
            let mut gui = make();
            let was = seen(&gui);
            let spoke = update(&mut gui, Message::EscapeCaptured).units() > 0;
            let now = seen(&gui);
            if now.focus != was.focus {
                captured.insert(Effect::Focus);
            }
            if !was.search.is_empty() && now.search.is_empty() {
                captured.insert(Effect::Clear);
            }
            honoured |= spoke;
        }
        if honoured {
            match out.iter_mut().find(|(name, _)| name == "Esc") {
                Some((_, all)) => all.extend(captured),
                None => out.push(("Esc".to_string(), captured)),
            }
        }
        out
    }

    /// The name a swept keystroke is listed under: the key, plus the modifiers
    /// the dispatcher actually reads for it.
    ///
    /// This is the one place the sweep's naming decision is made, and it is a
    /// claim about the dispatcher rather than about the sheet — so it is stated
    /// as such. `shortcut` reads `control` to pick its character arm and
    /// `shift` in exactly two places: on `Tab`, to walk focus the other way,
    /// and on `R`, to tell reset all from reset section. Every other modifier
    /// reaches the same arm as the bare key, so it is that key's binding reached
    /// another way and not a separate one to list. `Alt` is not swept at all:
    /// the dispatcher never reads it, so no binding can turn on it.
    ///
    /// The sweep presses all four combinations regardless (see [`keys`]), so
    /// this narrows only what the sheet has to say, never what the sweep can
    /// see. A binding added on a modifier the dispatcher does read — Shift on
    /// some other key, say — arrives under a name here that no line carries, and
    /// fails.
    fn binding_name(swept: &str) -> String {
        // Peel every leading modifier off, so `Ctrl+Shift+F` is read as both
        // modifiers on the key F rather than one modifier on the key `Shift+F`.
        let mut mods = String::new();
        let mut key = swept;
        while let Some(rest) = key.strip_prefix("Ctrl+") {
            mods.push_str("Ctrl");
            key = rest;
        }
        while let Some(rest) = key.strip_prefix("Shift+") {
            mods.push_str("Shift");
            key = rest;
        }
        match key {
            // Shift is read on Tab and on R, and nowhere else.
            "Tab" if mods.contains("Shift") => "Shift+Tab".to_string(),
            "R" if mods.contains("Shift") && mods.contains("Ctrl") => "Ctrl+Shift+R".to_string(),
            // A character is read through the Ctrl arm, so Ctrl is part of its
            // name; Shift on any other one collapses back to the key.
            key if key.chars().count() == 1 && mods.contains("Ctrl") => format!("Ctrl+{key}"),
            key => key.to_string(),
        }
    }

    /// The sheet's lines, parsed: the keys each names, and the words it spends
    /// on them. `Esc` names itself twice, and both lines count — they are two
    /// of the things one key does.
    fn lines() -> Vec<(Vec<String>, Vec<&'static str>)> {
        SHORTCUTS
            .lines()
            .skip(1)
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let (keys, words) = line.split_once("  ").unwrap_or_else(|| {
                    panic!("a line with a key column and a word column: {line:?}")
                });
                (
                    keys.split('/').flat_map(spelled_keys).collect(),
                    words.split(['/', ' ']).filter(|w| !w.is_empty()).collect(),
                )
            })
            .collect()
    }

    /// One column entry, expanded to the keys it names: `Tab` is one key,
    /// `Tab/Shift+Tab` two (the split above already did that), and
    /// `Ctrl+1..5` the five Ctrl+digit bindings there are. A range is a
    /// convenience for the sheet, never a key of its own.
    fn spelled_keys(column: &str) -> Vec<String> {
        match column.split_once("..") {
            Some((first, last)) => {
                // `Ctrl+1..5`: the digits are the range, the rest is the prefix.
                // Both ends are read, so a future `Ctrl+2..4` ranges over 2..=4
                // rather than silently starting at 1.
                let split =
                    first.len() - first.chars().rev().take_while(char::is_ascii_digit).count();
                let (prefix, start) = first.split_at(split);
                let start = start
                    .parse::<u8>()
                    .expect("the range starts with a digit")
                    .min(last.parse::<u8>().expect("a digit ending a range"));
                (start..=last.parse::<u8>().expect("a digit ending a range"))
                    .map(|n| format!("{prefix}{n}"))
                    .collect()
            }
            None => vec![column.to_string()],
        }
    }

    /// Every binding the dispatcher honours is named on the sheet, every key
    /// the sheet names is one the dispatcher honours, and every effect a key
    /// was seen to have is a word its line spends. This is the check that stops
    /// the list rotting: a binding added anywhere in `update.rs` lands here with
    /// no line and fails.
    ///
    /// Both directions matter, and the second is the stricter one. A missing
    /// line is an omission a reader can work around; a line naming a binding
    /// the app does not have sends them looking for a key that does nothing,
    /// which is the failure this exists to prevent.
    ///
    /// `Enter/Space` needs two lines because it does two different jobs: on a
    /// field row it flips, steps or types, and on a chrome control it saves,
    /// resets all or jumps to a section depending on which button the ring is
    /// on. The sheet says both on two lines; the sweep sees both across the
    /// field and chrome states.
    #[test]
    fn every_binding_the_window_honours_is_on_the_sheet() {
        let bindings = bindings();
        let sheet = lines();

        for (spelled, effects) in &bindings {
            let matching: Vec<_> = sheet
                .iter()
                .filter(|(keys, _)| keys.contains(spelled))
                .collect();
            assert!(
                !matching.is_empty(),
                "{spelled} is honoured and the sheet does not name it:\n{SHORTCUTS}"
            );
            let said: BTreeSet<Effect> = matching
                .iter()
                .flat_map(|(_, words)| words)
                .flat_map(|word| match claimed(word) {
                    Some(Claim::Effects(effects)) => Some(effects),
                    Some(Claim::HandedOver) => None,
                    None => panic!("{word:?} is not a claim the window can make"),
                })
                .flatten()
                .collect();
            assert!(
                effects.iter().all(|effect| said.contains(effect)),
                "{spelled}: the window does {effects:?} and the sheet says {said:?}"
            );
        }

        for (keys, _) in &sheet {
            for key in keys {
                assert!(
                    bindings.iter().any(|(name, _)| name == key),
                    "{key} is on the sheet and the window honours no such key"
                );
            }
        }
    }

    /// No line is wider than the sidebar, and the two columns line up: a line
    /// that overflows runs past its own panel and draws over the content pane
    /// behind it, which no pixel comparison elsewhere in the tree would catch.
    ///
    /// The bound is the measured one, not a guess: the sidebar is a fixed
    /// 160 px with 8 px of padding either side, and at the sheet's 10 px the
    /// widest line that fits draws to x=140. One character more and it does not.
    #[test]
    fn the_sheet_fits_the_sidebar() {
        const MAX_COLS: usize = 27;
        // The heading carries no columns: it is the sheet's own title.
        for line in SHORTCUTS.lines().skip(1).filter(|l| !l.trim().is_empty()) {
            let width = line.chars().count();
            assert!(
                width <= MAX_COLS,
                "{line:?} is {width} wide, the sidebar holds {MAX_COLS}"
            );
            let (keys, words) = line
                .split_once("  ")
                .unwrap_or_else(|| panic!("a line with two columns: {line:?}"));
            assert!(
                keys.chars().count() <= 13,
                "{keys:?} is longer than the key column"
            );
            assert!(
                !words.is_empty(),
                "{keys:?} says nothing about what it does"
            );
        }
    }

    /// The one claim the state diff cannot make: which way a step went. `back`
    /// and `forward` are separate words on the sheet's two arrow lines, so the
    /// arrows are driven on an option select and read off the same option table
    /// the renderer paints, rather than off the value alone.
    #[test]
    fn an_arrow_steps_the_way_its_line_says() {
        for (key, claimed) in [
            ("Left", "back"),
            ("Down", "back"),
            ("Right", "forward"),
            ("Up", "forward"),
        ] {
            let mut gui = test_gui("");
            gui.config.anchor = AnchorMode::Top;
            gui.focus = Some(FocusTarget::Field(Key::Anchor));
            let message = keys()
                .into_iter()
                .find(|(name, _)| name == key)
                .map(|(_, message)| message)
                .unwrap_or_else(|| panic!("{key} is a key the sweep presses"));
            let table = options(&gui, Key::Anchor);
            let at = table
                .iter()
                .position(|value| *value == Value::Anchor(gui.config.anchor))
                .expect("the row's value is one of its options");
            let _ = update(&mut gui, message);
            // The option table runs auto, top, bottom — the order the chips paint in.
            // Both arms named, so a typo in the table's own cases is a compile
            // error rather than a step landing the wrong way.
            match claimed {
                "back" => assert_eq!(
                    gui.config.anchor,
                    AnchorMode::Auto,
                    "{key} steps back from top to auto"
                ),
                "forward" => assert_eq!(
                    gui.config.anchor,
                    AnchorMode::Bottom,
                    "{key} steps forward from top to bottom"
                ),
                other => panic!("{other} is not a direction an arrow steps"),
            }
            let landed = table
                .iter()
                .position(|value| *value == Value::Anchor(gui.config.anchor))
                .expect("the row's value is one of its options");
            assert_eq!(
                landed.abs_diff(at),
                1,
                "{key} moves one option, not to an end"
            );
        }
    }
}
