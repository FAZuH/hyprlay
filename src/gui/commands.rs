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
use super::send;

/// Commit one numeric value to the mirror and the daemon. The caller has
/// already made sure the value is inside the key's bounds.
pub(super) fn apply_num(gui: &mut Gui, key: Key, value: i64) -> Task<Message> {
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
pub(super) fn mark_dirty(gui: &mut Gui, command: &Command) {
    if !hyprlay_core::domain::should_persist(command, gui.config.auto_save) {
        gui.dirty = true;
    }
}

pub(super) fn num_in_bounds(key: Key, v: i64) -> bool {
    key.num_bounds()
        .is_some_and(|(min, max)| v >= min && v <= max)
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
        | Message::Navigate(_)
        | Message::Scrolled(_)
        | Message::Measured { .. }
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
    use super::*;

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
