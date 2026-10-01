//! Daemon lifecycle awareness for the settings window: the status state
//! machine behind the bottom-left chip plus the Start/Stop toggle plumbing.
//!
//! State model: [`DaemonState::Connecting`] until the first probe answer;
//! any `status=` reply proves a live daemon ([`DaemonState::Up`], carrying
//! the reply verbatim); a failed probe ([`DaemonState::Unreachable`] —
//! nothing answered the control socket) shows as "daemon not active".
//! Every other reply (config echoes, `saved`, `quitting`, validation
//! errors from a *live* daemon) leaves the state untouched — only probe
//! outcomes may move it.
//!
//! The toggle→action *routing* (which [`Action`](hyprlay_core::daemon_control::Action)
//! a press resolves to, and how each is performed) lives in
//! `hyprlay_core::daemon_control`; this module owns only the GUI's view of
//! daemon state and the boot auto-start watcher.

use hyprlay_core::daemon_control::Toggle;
use hyprlay_core::domain::Reply;
use hyprlay_core::status::StatusFields;

/// Chip text before the first probe has answered.
pub(super) const CONNECTING_TEXT: &str = "connecting…";
const DAEMON_DOWN_TEXT: &str = "daemon not active";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DaemonState {
    Connecting,
    Up(Reply),
    Unreachable,
}

impl DaemonState {
    /// Fold one daemon reply into the machine. Only probe outcomes move it:
    /// a `status=` line proves the daemon answers, the send-wrapper's
    /// `Reply::Error` proves nothing does, and everything else is an
    /// ordinary reply that must not disturb the chip.
    pub(super) fn advance(self, reply: &Reply) -> Self {
        match reply {
            Reply::Ok(txt) if StatusFields::is_status_line(txt) => Self::Up(reply.clone()),
            Reply::Error(txt)
                if txt == crate::gui::DAEMON_UNREACHABLE || txt == super::COMMAND_TASK_FAILED =>
            {
                Self::Unreachable
            }
            _ => self,
        }
    }

    pub(super) fn text(&self) -> &str {
        match self {
            Self::Connecting => CONNECTING_TEXT,
            Self::Up(reply) => reply.text(),
            Self::Unreachable => DAEMON_DOWN_TEXT,
        }
    }

    /// Bottom-bar toggle caption. While connecting the state is unknown;
    /// the start affordance is the sensible default and renders dimmed.
    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Up(_) => "Stop daemon",
            Self::Connecting | Self::Unreachable => "Start daemon",
        }
    }

    /// What pressing the toggle means right now; `None` disables the
    /// button because no probe has answered yet.
    pub(super) fn toggle(&self) -> Option<Toggle> {
        match self {
            Self::Up(_) => Some(Toggle::Stop),
            Self::Unreachable => Some(Toggle::Start),
            Self::Connecting => None,
        }
    }
}

/// Does this reply prove that no daemon answered? Only the send-wrapper's
/// two failure texts may mark the daemon down; the daemon never sends
/// either. Decided by the `Reply::Error` variant the wrapper produces, so
/// no caller re-derives it from the text.
fn is_probe_failure(reply: &Reply) -> bool {
    matches!(reply, Reply::Error(_))
}

/// Boot watcher behind the one-shot auto-start: opening the GUI must bring
/// the daemon up when it is down. This type only owns WHEN; the start
/// itself goes through [`DaemonControl`] (the same path as the Start
/// toggle), so both surfaces can never diverge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No verdict yet; the first failed probe fires the launch.
    Watching,
    /// Fired; the blocking bring-up call is still out.
    Running,
    /// Settled — the launch returned or was never needed.
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct AutoStart(Phase);

impl AutoStart {
    pub(super) fn watching() -> Self {
        Self(Phase::Watching)
    }

    /// Fold one reply into `state` and decide whether this is the moment to
    /// auto-start: `Some(Toggle::Start)` exactly when a failed probe first
    /// proves the daemon down at boot. While the launched call is still
    /// out, later failures hold [`DaemonState::Connecting`] — they mean
    /// "not up yet", not "dead".
    pub(super) fn observe(&mut self, state: &mut DaemonState, reply: &Reply) -> Option<Toggle> {
        if self.0 == Phase::Running {
            // Hold the connecting line until the launch settles.
            if let Reply::Ok(txt) = reply
                && StatusFields::is_status_line(txt)
            {
                *state = state.clone().advance(reply);
                self.0 = Phase::Done;
            }
            return None;
        }
        if self.0 == Phase::Watching && is_probe_failure(reply) {
            // First proof of a down daemon at boot: fire once and keep
            // `connecting…` on screen until the attempt settles.
            self.0 = Phase::Running;
            return Some(Toggle::Start);
        }
        *state = state.clone().advance(reply);
        if matches!(state, DaemonState::Up(_)) {
            self.0 = Phase::Done;
        }
        None
    }

    /// The launched bring-up returned (either way); from here on probe
    /// outcomes speak for themselves again.
    pub(super) fn settled(&mut self) {
        if self.0 == Phase::Running {
            self.0 = Phase::Done;
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn first_status_reply_moves_connecting_to_up() {
        let reply = "status=connected channel=ngobrol 3 participants=2 rtl=on monitor=eDP-1";
        let next = DaemonState::Connecting.advance(&Reply::Ok(reply.into()));
        assert_eq!(next, DaemonState::Up(Reply::Ok(reply.into())));
    }

    #[test]
    fn any_later_status_reply_keeps_or_restores_up() {
        let reply = "status=disconnected";
        assert_eq!(
            DaemonState::Up(Reply::Ok(
                "status=connected channel=a participants=1".into()
            ))
            .advance(&Reply::Ok(reply.into())),
            DaemonState::Up(Reply::Ok(reply.into()))
        );
        assert_eq!(
            DaemonState::Unreachable.advance(&Reply::Ok(reply.into())),
            DaemonState::Up(Reply::Ok(reply.into()))
        );
    }

    #[test]
    fn unreachable_probe_marks_the_daemon_down_from_any_state() {
        assert_eq!(
            DaemonState::Connecting.advance(&Reply::Error(crate::gui::DAEMON_UNREACHABLE.into())),
            DaemonState::Unreachable
        );
        assert_eq!(
            DaemonState::Up(Reply::Ok(
                "status=connected channel=a participants=1".into()
            ))
            .advance(&Reply::Error(crate::gui::DAEMON_UNREACHABLE.into())),
            DaemonState::Unreachable
        );
    }

    #[test]
    fn dead_probe_task_counts_as_unreachable_too() {
        assert_eq!(
            DaemonState::Up(Reply::Ok(
                "status=connected channel=a participants=1".into()
            ))
            .advance(&Reply::Error(crate::gui::COMMAND_TASK_FAILED.into())),
            DaemonState::Unreachable
        );
    }

    #[test]
    fn replies_that_are_not_probe_outcomes_leave_the_state_untouched() {
        let bystanders = [
            "[position]\nwidth = 500",
            "saved",
            "quitting",
            "opacity=70",
            "",
        ];
        for reply in bystanders {
            let reply = Reply::Ok(reply.into());
            assert_eq!(
                DaemonState::Connecting.advance(&reply),
                DaemonState::Connecting,
                "reply {reply:?} must not leave connecting"
            );
            assert_eq!(
                DaemonState::Unreachable.advance(&reply),
                DaemonState::Unreachable,
                "reply {reply:?} must not revive a down daemon"
            );
            assert_eq!(
                DaemonState::Up(Reply::Ok(
                    "status=connected channel=a participants=1".into()
                ))
                .advance(&reply),
                DaemonState::Up(Reply::Ok(
                    "status=connected channel=a participants=1".into()
                )),
                "reply {reply:?} must not drop an up daemon"
            );
        }
    }

    #[test]
    fn validation_errors_from_a_live_daemon_do_not_mean_down() {
        let next = DaemonState::Up(Reply::Ok(
            "status=connected channel=a participants=1".into(),
        ))
        .advance(&Reply::Error("error: opacity <0-100>".into()));
        assert_eq!(
            next,
            DaemonState::Up(Reply::Ok(
                "status=connected channel=a participants=1".into()
            ))
        );
    }

    #[test]
    fn an_up_daemon_offers_stop() {
        let up = DaemonState::Up(Reply::Ok(
            "status=connected channel=a participants=1".into(),
        ));
        assert_eq!(up.label(), "Stop daemon");
        assert_eq!(up.toggle(), Some(Toggle::Stop));
    }

    #[test]
    fn a_down_daemon_offers_start() {
        assert_eq!(DaemonState::Unreachable.label(), "Start daemon");
        assert_eq!(DaemonState::Unreachable.toggle(), Some(Toggle::Start));
    }

    #[test]
    fn while_connecting_the_toggle_is_disabled_under_a_dimmed_start_label() {
        assert_eq!(DaemonState::Connecting.label(), "Start daemon");
        assert_eq!(DaemonState::Connecting.toggle(), None);
    }

    #[test]
    fn first_failed_probe_at_boot_fires_exactly_one_start_and_holds_connecting() {
        let mut watcher = AutoStart::watching();
        let mut state = DaemonState::Connecting;

        let fired = watcher.observe(
            &mut state,
            &Reply::Error(crate::gui::DAEMON_UNREACHABLE.into()),
        );

        assert_eq!(fired, Some(Toggle::Start));
        assert_eq!(state, DaemonState::Connecting);
        assert_eq!(
            watcher.observe(
                &mut state,
                &Reply::Error(crate::gui::DAEMON_UNREACHABLE.into())
            ),
            None,
            "the launch is one-shot per GUI session"
        );
    }

    #[test]
    fn failed_probes_while_the_launch_runs_hold_connecting() {
        // Until the bring-up returns, a failed probe only proves "not up
        // yet"; declaring "daemon not active" mid-launch would be wrong.
        let mut watcher = AutoStart::watching();
        let mut state = DaemonState::Connecting;
        assert_eq!(
            watcher.observe(
                &mut state,
                &Reply::Error(crate::gui::DAEMON_UNREACHABLE.into())
            ),
            Some(Toggle::Start)
        );

        let fired_again = watcher.observe(
            &mut state,
            &Reply::Error(crate::gui::COMMAND_TASK_FAILED.into()),
        );

        assert_eq!(fired_again, None);
        assert_eq!(state, DaemonState::Connecting);
    }

    #[test]
    fn a_daemon_already_up_at_boot_never_fires_a_launch() {
        let mut watcher = AutoStart::watching();
        let mut state = DaemonState::Connecting;
        let reply = "status=disconnected";

        let fired = watcher.observe(&mut state, &Reply::Ok(reply.into()));

        assert_eq!(fired, None);
        assert_eq!(state, DaemonState::Up(Reply::Ok(reply.into())));
    }

    #[test]
    fn ordinary_replies_at_boot_neither_fire_nor_move_the_state() {
        let mut watcher = AutoStart::watching();
        let mut state = DaemonState::Connecting;

        let fired = watcher.observe(&mut state, &Reply::Ok("saved".into()));

        assert_eq!(fired, None);
        assert_eq!(state, DaemonState::Connecting);
        assert_eq!(
            watcher.observe(
                &mut state,
                &Reply::Error(crate::gui::DAEMON_UNREACHABLE.into())
            ),
            Some(Toggle::Start),
            "the watcher stays armed until a probe gives a verdict"
        );
    }

    #[test]
    fn successful_probe_during_the_launch_marks_up_and_retires_the_watcher() {
        let mut watcher = AutoStart::watching();
        let mut state = DaemonState::Connecting;
        watcher.observe(
            &mut state,
            &Reply::Error(crate::gui::DAEMON_UNREACHABLE.into()),
        );

        let fired = watcher.observe(
            &mut state,
            &Reply::Ok("status=connected channel=a participants=1".into()),
        );

        assert_eq!(fired, None);
        assert!(matches!(state, DaemonState::Up(_)));
        // Retired: probes speak for themselves again.
        watcher.observe(
            &mut state,
            &Reply::Error(crate::gui::DAEMON_UNREACHABLE.into()),
        );
        assert_eq!(state, DaemonState::Unreachable);
    }

    #[test]
    fn after_the_launch_settles_a_failed_probe_reports_down_again() {
        let mut watcher = AutoStart::watching();
        let mut state = DaemonState::Connecting;
        assert_eq!(
            watcher.observe(
                &mut state,
                &Reply::Error(crate::gui::DAEMON_UNREACHABLE.into())
            ),
            Some(Toggle::Start)
        );
        watcher.settled();

        watcher.observe(
            &mut state,
            &Reply::Error(crate::gui::DAEMON_UNREACHABLE.into()),
        );

        assert_eq!(
            state,
            DaemonState::Unreachable,
            "once the launch returned, a dead daemon must be reported"
        );
    }
}
