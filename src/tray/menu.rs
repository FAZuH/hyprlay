//! Pure tray menu model and status-reply parsing.
//!
//! Everything here is free of `ksni`, `tray-icon`, and any concrete backend so
//! it can be unit-tested in isolation. The platform adapters in
//! [`crate::platform::tray`] map [`build_menu`] onto real menu items, all
//! driven through the shared [`Tray`](crate::tray::Tray) port.

use hyprlay_core::status::StatusFields;

/// A snapshot of daemon state the tray renders. Doubles as the diff-gate
/// key: two identical snapshots must not trigger a `handle.update`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayState {
    /// Whether a live daemon answered the `status` round-trip.
    pub up: bool,
    /// Overlay visibility as reported by the daemon.
    pub visible: bool,
    /// Compact, human-readable summary for the greyed status row.
    pub summary: String,
}

impl TrayState {
    /// The idle state shown before the first probe, or after the socket
    /// connect fails: no daemon reachable.
    pub fn down() -> Self {
        Self {
            up: false,
            visible: false,
            summary: "daemon: down".to_string(),
        }
    }

    /// Snapshot from a parsed status reply. Field semantics used:
    /// - `status` word — `connected` (and anything non-empty besides `off` /
    ///   `disconnected`) means up; `off` / `disconnected` means down.
    /// - `visible` — `on` / `off`.
    /// - `channel` / `participants` — feed the compact summary.
    fn from_fields(fields: &StatusFields) -> Self {
        let up = !fields.status_word.is_empty()
            && fields.status_word != "off"
            && fields.status_word != "disconnected";
        let summary = if up {
            // Connected: mirror the GUI's compact summary, adding the
            // participant count. Exact copy decided here (spec: "exact copy
            // finalised in code"); follows the Decisions worked example
            // `connected · #general · 3`.
            if fields.channel.is_empty() {
                fields.status_word.clone()
            } else {
                format!(
                    "{} · {} · {}",
                    fields.status_word, fields.channel, fields.participants
                )
            }
        } else {
            // Down-but-replied (e.g. `status=disconnected`): the word is the
            // whole story; a missing word falls back to the down default.
            if fields.status_word.is_empty() {
                "daemon: down".to_string()
            } else {
                fields.status_word.clone()
            }
        };
        TrayState {
            up,
            visible: fields.visible,
            summary,
        }
    }
}

/// Which user-facing action a menu item triggers. Sent over the action
/// channel by the menu's activate closures; the poll loop performs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Open the settings window (`hyprlay gui`).
    OpenSettings,
    /// Flip the overlay visibility over the ctl socket.
    ToggleVisible,
    /// Start or stop the daemon (routing decided by live state).
    ToggleDaemon,
    /// Exit the tray process only.
    Quit,
}

/// One declarative menu row. `ksni`-free so the menu is testable without a
/// DBus connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuRow {
    /// Greyed informational row; the label is the status summary.
    Status(String),
    /// A separator line.
    Separator,
    /// An activatable item carrying its label and the action it sends.
    Item { label: String, action: MenuAction },
}

/// Build the full menu model from the current state. Pure: the same state
/// always yields the same description, and labels mirror the live state
/// (Start/Stop, Show/Hide).
pub fn build_menu(state: &TrayState) -> Vec<MenuRow> {
    vec![
        MenuRow::Status(state.summary.clone()),
        MenuRow::Separator,
        MenuRow::Item {
            label: "Open settings".into(),
            action: MenuAction::OpenSettings,
        },
        MenuRow::Item {
            label: if state.visible {
                "Hide overlay"
            } else {
                "Show overlay"
            }
            .into(),
            action: MenuAction::ToggleVisible,
        },
        MenuRow::Item {
            label: if state.up {
                "Stop daemon"
            } else {
                "Start daemon"
            }
            .into(),
            action: MenuAction::ToggleDaemon,
        },
        MenuRow::Separator,
        MenuRow::Item {
            label: "Quit".into(),
            action: MenuAction::Quit,
        },
    ]
}

/// Parse a `status=` reply into a [`TrayState`]. The wire format itself is
/// owned by [`StatusFields`] — the single source of truth shared with the
/// daemon writer and the GUI reader — so channel names with spaces and
/// lenient numbers are handled there; this only maps the parsed fields onto
/// what the tray renders.
pub fn parse_status(reply: &str) -> TrayState {
    match StatusFields::parse_wire(reply) {
        Some(fields) => TrayState::from_fields(&fields),
        None => TrayState::down(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn down_state_is_the_idle_summary() {
        let state = TrayState::down();
        assert!(!state.up);
        assert!(!state.visible);
        assert_eq!(state.summary, "daemon: down");
    }

    #[test]
    fn menu_shows_start_and_show_when_daemon_is_down() {
        let rows = build_menu(&TrayState::down());
        assert_eq!(rows[0], MenuRow::Status("daemon: down".into()));
        assert!(matches!(rows[1], MenuRow::Separator));
        // Open settings, then Show overlay (down => not visible), then
        // Start daemon (down => Start), then Quit.
        let labels: Vec<_> = rows
            .iter()
            .filter_map(|r| match r {
                MenuRow::Item { label, .. } => Some(label.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            labels,
            ["Open settings", "Show overlay", "Start daemon", "Quit"]
        );
    }

    #[test]
    fn menu_mirrors_up_and_visible_state() {
        let state = TrayState {
            up: true,
            visible: true,
            summary: "connected · #general · 3".into(),
        };
        let rows = build_menu(&state);
        assert_eq!(rows[0], MenuRow::Status("connected · #general · 3".into()));
        let labels: Vec<_> = rows
            .iter()
            .filter_map(|r| match r {
                MenuRow::Item { label, .. } => Some(label.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            labels,
            ["Open settings", "Hide overlay", "Stop daemon", "Quit"]
        );
    }

    #[test]
    fn parse_status_up_connected_with_channel_and_participants() {
        let reply = "status=connected channel=#general participants=3 rtl=on monitor=eDP-1";
        let state = parse_status(reply);
        assert!(state.up);
        assert!(!state.visible);
        assert_eq!(state.summary, "connected · #general · 3");
    }

    #[test]
    fn parse_status_visible_flag_is_read() {
        let state = parse_status("status=connected channel=a participants=2 visible=on");
        assert!(state.up);
        assert!(state.visible);
    }

    #[test]
    fn parse_status_disconnected_is_down_with_word_summary() {
        let state = parse_status("status=disconnected");
        assert!(!state.up);
        assert_eq!(state.summary, "disconnected");
    }

    #[test]
    fn parse_status_off_word_is_treated_as_down() {
        let state = parse_status("status=off channel=- participants=0 visible=off");
        assert!(!state.up);
        assert_eq!(state.summary, "off");
    }

    #[test]
    fn parse_status_without_status_prefix_is_down() {
        // A connect error (ctl::send_command_line returned None upstream) or
        // any non-status reply must not be mistaken for an up daemon.
        assert_eq!(parse_status("error: daemon unreachable"), TrayState::down());
        assert_eq!(parse_status(""), TrayState::down());
    }

    #[test]
    fn parse_status_keeps_multiword_channel_intact() {
        // Channels may contain spaces; the summary must not split mid-name.
        let state = parse_status("status=connected channel=my cool guild participants=12");
        assert_eq!(state.summary, "connected · my cool guild · 12");
    }

    #[test]
    fn parse_status_includes_the_participant_count() {
        let state = parse_status("status=connected channel=solo participants=1");
        assert_eq!(state.summary, "connected · solo · 1");
    }
}
