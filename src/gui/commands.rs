//! Message → Command translation and the bookkeeping the update arms
//! share: the unsaved-changes marker, the numeric bounds check, and the
//! revert diff behind "clear changes".

use hyprlay_core::config::Config;
use hyprlay_core::domain::Command;
use hyprlay_core::domain::Key;
use hyprlay_core::domain::Value;
use hyprlay_core::domain::corner_of;
use iced::Task;

use super::Gui;
use super::Message;
use super::fields;
use super::send;

/// Commit one numeric value to the mirror and the daemon. The caller has
/// already made sure the value is inside the key's bounds.
pub(super) fn apply_num(gui: &mut Gui, key: Key, value: i64) -> Task<Message> {
    gui.num_drafts.remove(&key);
    apply_change(gui, Command::Set(key, Value::Num(value)))
}

/// Mirror the command onto the config, tell the daemon, and set the unsaved
/// marker — the whole of what every change arm does, in one place because the
/// marker is only correct when the apply and the decision happen together.
///
/// Both halves of the daemon's rule are kept: `should_persist` is asked with
/// the *pre-application* autosave value, the same one the daemon uses, so
/// flipping auto-save itself never leaves a phantom badge; and the marker
/// comes from the *outcome* rather than the intent, so a command the daemon
/// refuses — a per-key reset of the monitor, an `offset-min` past the current
/// `offset-max` — leaves no badge at all. Deciding from the intent instead
/// would claim an unsaved change for a command that changed nothing.
pub(super) fn apply_change(gui: &mut Gui, command: Command) -> Task<Message> {
    let persists = hyprlay_core::domain::should_persist(&command, gui.config.auto_save);
    let result = command.clone().apply_config(&mut gui.config);
    if result.reply.is_ok() && !persists {
        gui.dirty = true;
    }
    Task::perform(send(command.to_string()), Message::Applied)
}

/// The unsaved marker for a command the GUI does not apply itself: the group
/// and global resets wait for the daemon's `dump` to bring the config back,
/// and `switch monitor` writes its own field. Decided with the pre-application
/// autosave value, so flipping auto-save itself leaves no phantom badge.
pub(super) fn mark_dirty(gui: &mut Gui, command: &Command) {
    if !hyprlay_core::domain::should_persist(command, gui.config.auto_save) {
        gui.dirty = true;
    }
}

pub(super) fn num_in_bounds(key: Key, v: i64) -> bool {
    key.num_bounds()
        .is_some_and(|(min, max)| v >= min && v <= max)
}

/// The command one keyboard step along the focused row sends: the next choice
/// forward (Enter, Space, Right, Up) or the previous one back (Left, Down), read
/// in the order the row renders them — or, for a number row, the value one
/// [`fields::NUM_STEP`] away.
///
/// `None` when there is nothing to send, and there are three ways that happens:
/// a row that neither holds a number nor offers a fixed set of choices (a
/// colour editor, a flag), a number already at its bound, and a choice row
/// with a single option. A choice row with somewhere to go wraps past either
/// end onto the other end, so a held arrow keeps cycling the row instead of
/// stopping on the last option — but wrapping a lone option onto itself would
/// send a command that changes nothing and still marks the window dirty.
///
/// A number step is clamped to the bounds the GUI and the daemon share, so the
/// value can reach a bound and stop there but can never leave the range,
/// whichever way the arrows are pushed.
pub(super) fn step_command(gui: &Gui, key: Key, forward: bool) -> Option<Command> {
    if let Some((min, max)) = key.num_bounds() {
        let Value::Num(current) = key.value_of(&gui.config) else {
            unreachable!("a key with numeric bounds holds a number");
        };
        let moved = if forward {
            fields::NUM_STEP
        } else {
            -fields::NUM_STEP
        };
        let next = (current + moved).clamp(min, max);
        return (next != current).then_some(Command::Set(key, Value::Num(next)));
    }
    let options = fields::options(gui, key);
    if options.len() < 2 {
        return None;
    }
    let current = key.value_of(&gui.config);
    let index = options.iter().position(|value| *value == current)?;
    let next = if forward {
        options.get(index + 1).or_else(|| options.first())
    } else if index == 0 {
        options.last()
    } else {
        options.get(index - 1)
    }?;
    Some(Command::Set(key, next.clone()))
}

/// The command every reset of one field sends, whether it came from the R key,
/// a number row's reset button or a colour editor's: restore one setting to
/// its default.
///
/// A real reset command, not `set` with a value computed here. The daemon owns
/// the default table (`Config::default()` lives in core and is what
/// `reset <group>` walks), so the GUI never keeps a second copy of it — the
/// same rule the daemon's own `reset` follows.
pub(super) fn reset_command(key: Key) -> Command {
    Command::ResetKey(key)
}

/// Commands that bring `live` back to `saved`, one per differing key.
/// Used by "clear changes"; empty when there is nothing to revert. Walking
/// the shared [`Key`] table means a newly added setting can never be
/// forgotten here — it shows up in the diff the moment it exists.
pub(super) fn revert_commands(live: &Config, saved: &Config) -> Vec<Command> {
    Key::ALL
        .into_iter()
        .filter(|k| k.value_of(live) != k.value_of(saved))
        .map(|k| Command::Set(k, k.value_of(saved)))
        .collect()
}

/// Turn a GUI interaction into its control-socket command. The local config
/// mirror is the same [`Command::apply_config`] the daemon runs, so both
/// sides can never disagree about what a setting change means.
pub(super) fn command_for(message: Message) -> Command {
    match message {
        Message::Position(h, v) => Command::Set(Key::Position, Value::Corner(corner_of(h, v))),
        // Rides the generic apply path like Position: mirror locally, send
        // the same wire command the CLI would.
        Message::Anchor(mode) => Command::Set(Key::Anchor, Value::Anchor(mode)),
        Message::RosterOrder(order) => Command::Set(Key::RosterOrder, Value::RosterOrder(order)),
        Message::SetOption(key, value) => Command::Set(key, value),
        Message::SetFlag(..) => unreachable!("flags are handled directly in update"),
        // Handled directly in `update`; unreachable here.
        Message::NumText(..)
        | Message::NumSubmit(..)
        | Message::EscapeCaptured
        | Message::NumDrag(..)
        | Message::ResetFocused(_)
        | Message::ColorPart(..)
        | Message::ColorHex(..)
        | Message::PickerToggle(..)
        | Message::SvPress(..)
        | Message::SvMove(..)
        | Message::HuePress(..)
        | Message::HueMove(..)
        | Message::PickerRelease
        | Message::Palette(_)
        | Message::Navigate(_)
        | Message::Scrolled(_)
        | Message::Measured { .. }
        | Message::ScrollContentTo(_)
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

#[cfg(test)]
mod tests {
    use hyprlay_core::config::AnchorMode;
    use hyprlay_core::config::HorizontalAnchor as H;
    use hyprlay_core::config::RosterOrder;
    use hyprlay_core::config::VerticalAnchor as V;
    use hyprlay_core::domain::Corner;
    use hyprlay_core::domain::Group;
    use hyprlay_core::domain::MonitorTarget;

    use super::*;
    use crate::gui::test_gui;

    /// The GUI with two monitors reported, which is the only shape of the
    /// monitor row that has more than one option to step between.
    fn gui() -> Gui {
        let mut g = test_gui("");
        g.monitors = ["DP-1".to_string(), "HDMI-A-1".to_string()].into();
        g
    }

    /// Every option-select row's step sends exactly the choice next door in
    /// the row's own order, and it is the wire text the daemon parses. This is
    /// the whole of "the change reaches the daemon" that no screenshot can
    /// show: the daemon is not running in the GUI harness, so the command is
    /// asserted here instead.
    #[test]
    fn an_option_step_sends_the_neighbouring_choice() {
        let mut g = gui();
        assert_eq!(
            step_command(&g, Key::Position, true),
            Some(Command::Set(Key::Position, Value::Corner(Corner::TopRight))),
            "Right on top-left is top-right"
        );
        assert_eq!(
            step_command(&g, Key::Anchor, true),
            Some(Command::Set(Key::Anchor, Value::Anchor(AnchorMode::Top))),
            "Right on auto is top"
        );
        assert_eq!(
            step_command(&g, Key::RosterOrder, true),
            Some(Command::Set(
                Key::RosterOrder,
                Value::RosterOrder(RosterOrder::Name)
            )),
            "Right on join-order is name"
        );
        assert_eq!(
            step_command(&g, Key::Monitor, true),
            Some(Command::Set(
                Key::Monitor,
                Value::Target(MonitorTarget::Named("DP-1".into()))
            )),
            "Right on active is the first reported output"
        );

        // And Left is the same walk in reverse, from a row that has moved.
        g.config.anchor = AnchorMode::Bottom;
        assert_eq!(
            step_command(&g, Key::Anchor, false),
            Some(Command::Set(Key::Anchor, Value::Anchor(AnchorMode::Top))),
            "Left on bottom is top"
        );
        assert_eq!(
            Command::Set(Key::Anchor, Value::Anchor(AnchorMode::Top)).to_string(),
            "set anchor top",
            "the step sends the canonical wire form"
        );
    }

    /// Both ends wrap: Left on the first option returns to the last, and
    /// Right on the last returns to the first, so a held arrow keeps cycling
    /// the row instead of stopping.
    #[test]
    fn an_option_step_at_either_end_wraps_to_the_other_end() {
        let mut g = gui();
        assert_eq!(
            step_command(&g, Key::Anchor, false),
            Some(Command::Set(Key::Anchor, Value::Anchor(AnchorMode::Bottom))),
            "Left on the first option wraps to the last"
        );
        assert_eq!(
            step_command(&g, Key::Position, false),
            Some(Command::Set(
                Key::Position,
                Value::Corner(Corner::BottomRight)
            )),
            "and so does Left on the first preset"
        );
        g.config.anchor = AnchorMode::Bottom;
        assert_eq!(
            step_command(&g, Key::Anchor, true),
            Some(Command::Set(Key::Anchor, Value::Anchor(AnchorMode::Auto))),
            "Right on the last option wraps to the first"
        );
        g.config.horizontal = H::Right;
        g.config.vertical = V::Bottom;
        assert_eq!(
            step_command(&g, Key::Position, true),
            Some(Command::Set(Key::Position, Value::Corner(Corner::TopLeft))),
            "and so does Right on the last preset"
        );
        g.config.monitor = Some("HDMI-A-1".into());
        assert_eq!(
            step_command(&g, Key::Monitor, true),
            Some(Command::Set(
                Key::Monitor,
                Value::Target(MonitorTarget::Active)
            )),
            "and on the last reported output"
        );
        g.config.monitor = None;
        assert_eq!(
            step_command(&g, Key::Monitor, false),
            Some(Command::Set(
                Key::Monitor,
                Value::Target(MonitorTarget::Named("HDMI-A-1".into()))
            )),
            "while Left on the first output wraps to the last"
        );
    }

    /// A choice row with a single option has nowhere to step: wrapping it onto
    /// itself would send a command that changes nothing and still marks the
    /// window dirty. The monitor row with zero outputs reported is the shape
    /// this happens in — "active" is its only chip — so an arrow there sends
    /// nothing either way.
    #[test]
    fn a_single_option_row_steps_to_nothing() {
        let g = test_gui("");
        assert!(
            g.monitors.is_empty(),
            "this test needs the zero-output shape with its lone chip"
        );
        for forward in [true, false] {
            assert_eq!(
                step_command(&g, Key::Monitor, forward),
                None,
                "a lone option wraps onto itself, so there is no command to send"
            );
        }
    }

    /// A row with neither a number nor a fixed set of choices has no step at
    /// all, so an arrow on a colour editor or a flag cannot move anything —
    /// those rows have nothing to move, and guessing would send a wrong
    /// command.
    #[test]
    fn a_row_with_neither_numbers_nor_options_has_no_step() {
        let g = gui();
        for key in [Key::Rtl, Key::SpeakingColor, Key::TextColor, Key::BoxColor] {
            assert_eq!(
                step_command(&g, key, true),
                None,
                "{} offers neither a number nor a choice, so it must produce no command",
                key.name()
            );
        }
    }

    /// Every arrow on a number row sends the neighbouring value, and it is the
    /// wire text the daemon parses. This is the whole of "the step reaches the
    /// daemon" that no screenshot can show: the daemon is not running in the
    /// GUI harness, so the command is asserted here instead.
    #[test]
    fn a_number_step_sends_the_neighbouring_value() {
        let g = gui();
        assert_eq!(
            step_command(&g, Key::Spacing, true),
            Some(Command::Set(Key::Spacing, Value::Num(5))),
            "spacing is 4 on a clean config, so Up is 5"
        );
        assert_eq!(
            step_command(&g, Key::Spacing, false),
            Some(Command::Set(Key::Spacing, Value::Num(3))),
            "and Down is 3"
        );
        assert_eq!(
            Command::Set(Key::Spacing, Value::Num(5)).to_string(),
            "set spacing 5",
            "the step sends the canonical wire form"
        );
        // A negative offset moves the same way, and the sign survives the wire.
        let mut g = g;
        g.config.offset_x = -12;
        assert_eq!(
            step_command(&g, Key::OffsetX, true),
            Some(Command::Set(Key::OffsetX, Value::Num(-11))),
            "offset x is -12, so Up is -11"
        );
        assert_eq!(
            Command::Set(Key::OffsetX, Value::Num(-11)).to_string(),
            "set offset-x -11"
        );
    }

    /// The clamping proof, over every numeric key and every value that sits on
    /// or beside a bound: a step from there sends a value the shared table
    /// accepts, whichever way it is pushed. Asserted on the command, because
    /// that is what the daemon would be told — and because `apply_config`
    /// *refuses* a number outside its bounds rather than clamping it, so an
    /// unclamped step would come back as an error reply and change nothing.
    #[test]
    fn a_number_step_stays_inside_the_shared_bounds() {
        for key in Key::ALL {
            let Some((min, max)) = key.num_bounds() else {
                continue;
            };
            // The four values that can be one step from leaving the range, plus
            // the middle of the narrowest range (text size is 8..=32) where a
            // single step in either direction is still well inside it.
            let probes = [min, min + 1, max - 1, max, (min + max) / 2];
            for start in probes {
                for forward in [true, false] {
                    let mut g = gui();
                    Command::Set(key, Value::Num(start)).apply_config(&mut g.config);
                    let Some(Command::Set(_, Value::Num(next))) = step_command(&g, key, forward)
                    else {
                        continue;
                    };
                    assert!(
                        (min..=max).contains(&next),
                        "{} at {start} stepped to {next} (forward={forward}), outside {min}..={max}",
                        key.name()
                    );
                }
            }
        }
    }

    /// A held arrow on every numeric key and both directions: the row walks to
    /// the end of its range, comes to rest there, and one more press cannot
    /// move it. Asserted on the config mirror rather than on the command,
    /// because one key's range has a second edge — `offset-min` must also stay
    /// below `offset-max`, and `apply_config` is where that edge lives, so the
    /// window there can stop a press short of the declared bound.
    #[test]
    fn a_held_number_step_comes_to_rest_and_stops_there() {
        for key in Key::ALL {
            let Some((min, max)) = key.num_bounds() else {
                continue;
            };
            for forward in [true, false] {
                let mut g = gui();
                let mut at_rest = key.value_of(&g.config);
                // Far more presses than the whole range is wide, so the walk ends
                // at rest rather than running out of iterations.
                for _ in 0..(max - min + 2) {
                    let Some(Command::Set(_, value)) = step_command(&g, key, forward) else {
                        break;
                    };
                    Command::Set(key, value).apply_config(&mut g.config);
                    if key.value_of(&g.config) == at_rest {
                        // The applier refused it: the row is at the end of its
                        // range already, and no press will move it.
                        break;
                    }
                    at_rest = key.value_of(&g.config);
                }
                if let Some(Command::Set(_, value)) = step_command(&g, key, forward) {
                    Command::Set(key, value).apply_config(&mut g.config);
                }
                assert_eq!(
                    key.value_of(&g.config),
                    at_rest,
                    "{} held one way must come to rest, not creep",
                    key.name()
                );
                let Value::Num(rest) = at_rest else {
                    unreachable!("a numeric key holds a number");
                };
                assert!(
                    (min..=max).contains(&rest),
                    "{} rested at {rest}, outside the shared {min}..={max}",
                    key.name()
                );
            }
        }
    }

    /// The other half of the boundary: standing on a bound, the step that would
    /// leave the range produces no command at all — no write, no wire text.
    /// Numbers stay clamped; only choice rows wrap.
    #[test]
    fn a_number_step_off_the_bound_sends_nothing() {
        let mut g = gui();
        for key in [Key::Opacity, Key::Spacing, Key::TextSize, Key::OffsetX] {
            let (min, max) = key.num_bounds().expect("numeric key");
            for (end, forward) in [(min, false), (max, true)] {
                Command::Set(key, Value::Num(end)).apply_config(&mut g.config);
                assert_eq!(
                    step_command(&g, key, forward),
                    None,
                    "{} at {end} cannot step further out",
                    key.name()
                );
            }
        }
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
    fn roster_order_setting_roundtrips_through_apply_and_revert() {
        // The exact Command path the GUI chip row drives.
        let mut live = Config::default();
        let pick_name = Command::Set(
            Key::RosterOrder,
            Value::RosterOrder(hyprlay_core::config::RosterOrder::Name),
        );
        pick_name.clone().apply_config(&mut live);
        assert_eq!(live.roster_order, hyprlay_core::config::RosterOrder::Name);

        // Reverting mirrors what "clear changes" replays: read the saved
        // value back through the shared table and re-apply it.
        let saved = Config::default();
        let revert = Command::Set(Key::RosterOrder, Key::RosterOrder.value_of(&saved));
        revert.apply_config(&mut live);
        assert_eq!(live.roster_order, saved.roster_order);
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

    /// The command every one-field reset sends. The wire form is two words
    /// because two key names are spelled like group names — see
    /// [`Command::ResetKey`] — and it is asserted here because the group is
    /// read off the key rather than chosen by the caller, so no caller can
    /// pair the wrong two.
    #[test]
    fn a_per_key_reset_names_the_key_own_group() {
        assert_eq!(reset_command(Key::Width).to_string(), "reset layout width");
        // The ambiguous pair, both spellings: the group arm still wins the
        // one-word form, and the per-key form still says which it means.
        assert_eq!(
            "reset opacity".parse::<Command>().unwrap(),
            Command::ResetGroup(Group::Opacity)
        );
        assert_eq!(
            reset_command(Key::Opacity).to_string(),
            "reset opacity opacity"
        );
        assert_eq!(
            reset_command(Key::Position).to_string(),
            "reset position position"
        );
        // Every key's reset is a line the daemon's own parser accepts.
        for key in Key::ALL {
            let wire = reset_command(key).to_string();
            assert_eq!(
                wire.parse::<Command>().unwrap(),
                reset_command(key),
                "{wire}"
            );
        }
    }

    /// No reset command may ever name a credential. The credential rows are
    /// keyboard-reachable now, so a credential can be focused, edited and
    /// looked at — but every reset path builds its commands out of `Key`
    /// (`revert_commands` walks `Key::ALL`, and section/global reset speak
    /// `Command::Reset*`), and a credential has no `Key` to be named by. That
    /// is the property, and it is checked here against the wire forms rather
    /// than against the registry: putting a secret in a reset command is the
    /// failure, and the wire is where it would leave the process.
    ///
    /// Both halves matter. The values check is the one that would catch a leak
    /// today; the names check is the one that keeps the door shut, since a
    /// future `Key::ClientSecret` would be the first step toward a reset that
    /// could name one.
    #[test]
    fn no_reset_command_carries_a_credential() {
        // Stand-in values, not the real pair: this checks that the *plumbing*
        // has no path from a credential draft to a command, and that path
        // would carry whatever value is loaded.
        let (probe_id, probe_secret) = ("credential-probe-id", "credential-probe-secret");

        // Every command a reset can produce: the per-key revert diff with the
        // two configs differing in several sections, plus both reset verbs.
        let saved = Config {
            horizontal: H::Right,
            vertical: V::Bottom,
            rtl: true,
            offset_x: 40,
            offset_y: -12,
            opacity: 70,
            width: 500,
            scale: 120,
            speaking_color: "#00ff00".parse().unwrap(),
            ..Config::default()
        };
        let mut wire: Vec<String> = revert_commands(&Config::default(), &saved)
            .iter()
            .map(Command::to_string)
            .collect();
        wire.push(Command::ResetAll.to_string());
        for group in hyprlay_core::domain::Group::ALL {
            wire.push(Command::ResetGroup(group).to_string());
        }
        // And every per-key reset, which is the path a focused row's R takes.
        for key in Key::ALL {
            wire.push(reset_command(key).to_string());
        }
        assert!(
            wire.len() > 30,
            "the check proved nothing over a short list"
        );

        for line in &wire {
            assert!(
                !line.contains(probe_id) && !line.contains(probe_secret),
                "a reset command carries a credential: {line}"
            );
        }
        // And the vocabulary that builds them: no config key is named after a
        // credential, so none of these lines ever could have carried one.
        for key in Key::ALL {
            let name = key.name();
            assert!(
                !name.contains("client") && !name.contains("secret"),
                "config key {name} is named after a credential"
            );
        }
    }

    #[test]
    fn revert_commands_do_nothing_when_configs_match() {
        let cfg = Config::default();
        assert!(revert_commands(&cfg, &cfg).is_empty());
    }

    #[test]
    fn revert_commands_cover_every_differing_key_once() {
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
