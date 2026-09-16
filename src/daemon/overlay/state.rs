//! Application state: the model the view renders and commands mutate.
//!
//! Owns the participant list, the avatar cache, and placement state. All
//! transitions run through methods (`apply_discord`, `apply_config`,
//! `reanchor`, ...) so callers never poke at fields directly; the shell only
//! translates the returned outcomes into iced Tasks.

use std::collections::HashMap;
use std::collections::HashSet;

use hyprlay_core::config::Config;
use hyprlay_core::config::RosterOrder;
use hyprlay_core::domain::ConnectionStatus;
use iced::widget::image::Handle;

use crate::daemon::adapters::avatar;
use crate::daemon::adapters::discord::DiscordEvent;
use crate::daemon::adapters::discord::Participant;

/// Extra padding around an avatar reserved for the speaking ring.
const RING_PADDING: f32 = 8.0;

/// In-memory avatar store: decoded handles keyed by user id, plus the set
/// of fetches already in flight so a roster burst never double-fetches.
#[derive(Default)]
struct AvatarCache {
    handles: HashMap<String, Handle>,
    requested: HashSet<String>,
}

impl AvatarCache {
    /// (user_id, avatar_hash, url) triples for users whose avatar is
    /// neither cached nor already in flight.
    fn missing(&self, users: &[Participant]) -> Vec<(String, String, String)> {
        users
            .iter()
            .filter_map(|p| {
                let hash = p.avatar_hash.as_ref()?;
                if self.requested.contains(&p.id) || self.handles.contains_key(&p.id) {
                    return None;
                }
                Some((p.id.clone(), hash.clone(), avatar::url_for(&p.id, hash)))
            })
            .collect()
    }

    fn mark_requested<I: IntoIterator<Item = String>>(&mut self, ids: I) {
        self.requested.extend(ids);
    }

    fn insert(&mut self, user_id: String, data: Vec<u8>) {
        self.handles.insert(user_id, Handle::from_bytes(data));
    }

    fn get(&self, user_id: &str) -> Option<&Handle> {
        self.handles.get(user_id)
    }

    /// Pull any disk-cached avatars into memory (keyed by user + hash, so
    /// stale entries can never be served).
    fn hydrate(&mut self, users: &[Participant]) {
        for p in users {
            let Some(hash) = &p.avatar_hash else { continue };
            if self.handles.contains_key(&p.id) {
                continue;
            }
            if let Some(bytes) = crate::daemon::adapters::cache::load_avatar(&p.id, hash) {
                self.insert(p.id.clone(), bytes);
            }
        }
    }
}

pub struct Overlay {
    config: Config,
    status: ConnectionStatus,
    me_id: Option<String>,
    channel_name: Option<String>,
    users: Vec<Participant>,
    /// Tick when each participant last started speaking. Runtime-only
    /// presentation state for the recent-speakers order — never serialized,
    /// and dropped the moment someone stops speaking or leaves the channel.
    speakers: HashMap<String, u64>,
    tick: u64,
    avatars: AvatarCache,
    /// Label of the authentication path for `hyprlay status`. Own-app
    /// credentials are the only path, so this never varies.
    auth_label: &'static str,
    /// (top, right, bottom, left) layer-shell margins — only anchored sides
    /// matter; derived from `offset_x`/`offset_y`.
    offset: (i32, i32, i32, i32),
    size: (u32, u32),
    hovered: bool,
}

/// What changed after applying a Discord event — the shell turns this into
/// Tasks (surface resize + avatar fetches).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RosterChange {
    Changed,
    Unchanged,
}

impl Overlay {
    pub fn new(config: Config) -> Self {
        let offset = super::geometry::offset(&config);
        let width = config.width;
        Self {
            config,
            status: ConnectionStatus::Connecting,
            me_id: None,
            channel_name: None,
            users: Vec::new(),
            speakers: HashMap::new(),
            tick: 0,
            avatars: AvatarCache::default(),
            auth_label: "own-app",
            offset,
            size: (width, 0),
            hovered: false,
        }
    }

    // -- read accessors ---------------------------------------------------

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Mutable escape hatch for command application and persistence.
    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    pub fn status(&self) -> ConnectionStatus {
        self.status
    }

    pub fn channel_name(&self) -> Option<&str> {
        self.channel_name.as_deref()
    }

    pub fn auth_label(&self) -> &'static str {
        self.auth_label
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    pub fn offset(&self) -> (i32, i32, i32, i32) {
        self.offset
    }

    pub fn is_hovered(&self) -> bool {
        self.hovered
    }

    pub fn set_hovered(&mut self, v: bool) -> bool {
        if self.hovered != v {
            self.hovered = v;
            true
        } else {
            false
        }
    }

    pub fn effective_alphas(&self) -> hyprlay_core::config::Alphas {
        self.config.alphas_for(self.hovered)
    }

    // -- discord events ----------------------------------------------------

    /// Apply one event from the Discord RPC stream. Roster persistence is a
    /// rule of the model itself: it writes to disk only while connected, so
    /// a monitor switch mid-outage can't cache an empty screen.
    pub fn apply_discord(&mut self, ev: DiscordEvent) -> RosterChange {
        match ev {
            DiscordEvent::Status(s) => {
                self.status = s;
                RosterChange::Unchanged
            }
            DiscordEvent::Me(id) => {
                self.me_id = Some(id);
                RosterChange::Unchanged
            }
            DiscordEvent::Channel(name) => {
                self.channel_name = name;
                RosterChange::Unchanged
            }
            DiscordEvent::Participants(users) => {
                self.track_speakers(&users);
                self.users = users;
                if self.status == ConnectionStatus::Connected {
                    crate::daemon::adapters::cache::save_roster(
                        self.channel_name.as_deref(),
                        self.me_id.as_deref(),
                        &self.users,
                    );
                }
                self.avatars.hydrate(&self.users);
                RosterChange::Changed
            }
        }
    }

    // -- derived views ------------------------------------------------------

    /// The displayed rows, cut off at `max_rows` (0 = unlimited). Rows past
    /// the cap are not rendered; [`Overlay::hidden_rows`] counts them.
    pub fn displayed(&self) -> Vec<&Participant> {
        let mut rows = self.eligible();
        if self.config.max_rows > 0 {
            rows.truncate(self.config.max_rows as usize);
        }
        rows
    }

    /// Participants that overflow the row cap — the "+N" pill's N. Rows
    /// removed by the config filters do not count: they are not hidden by
    /// the cap.
    pub fn hidden_rows(&self) -> usize {
        if self.config.max_rows == 0 {
            return 0;
        }
        self.eligible()
            .len()
            .saturating_sub(self.config.max_rows as usize)
    }

    /// Participants after applying the config filters (own user, talking)
    /// and the configured roster order, before the row cap. Hidden
    /// short-circuits to an empty list so the surface collapses through the
    /// normal empty-surface path — no layer-shell unmap games; the daemon
    /// keeps tracking state while invisible.
    fn eligible(&self) -> Vec<&Participant> {
        if !self.config.visible {
            return Vec::new();
        }
        let mut rows: Vec<&Participant> = self
            .users
            .iter()
            .filter(|p| self.config.show_own_user || Some(&p.id) != self.me_id.as_ref())
            .filter(|p| !self.config.show_only_talking_users || p.speaking)
            .collect();
        self.sort_rows(&mut rows);
        rows
    }

    /// One ordering pass over already-filtered rows. Every strategy is a
    /// stable sort, so ties fall back to join order everywhere.
    fn sort_rows(&self, rows: &mut [&Participant]) {
        match self.config.roster_order {
            RosterOrder::JoinOrder => {}
            RosterOrder::Name => rows.sort_by_key(|p| p.name.to_lowercase()),
            RosterOrder::RecentSpeakers => {
                rows.sort_by(|a, b| self.speakers.get(&b.id).cmp(&self.speakers.get(&a.id)))
            }
        }
    }

    /// Diff speaking flags against the previous roster: a start stamps the
    /// participant with the next tick (the top of the recent-speakers
    /// order), a stop drops the record so they sink back to join order.
    /// Departed participants lose their record with the roster.
    fn track_speakers(&mut self, users: &[Participant]) {
        self.tick += 1;
        for p in users {
            let was_speaking = self.users.iter().any(|u| u.id == p.id && u.speaking);
            match (was_speaking, p.speaking) {
                (false, true) => {
                    self.speakers.insert(p.id.clone(), self.tick);
                }
                (true, false) => {
                    self.speakers.remove(&p.id);
                }
                _ => {}
            }
        }
        self.speakers
            .retain(|id, _| users.iter().any(|u| &u.id == id));
    }

    /// Surface size (logical px) for the currently displayed rows. Height 0
    /// means "nothing to show". A truncated roster reserves one more row
    /// for the "+N" overflow pill.
    pub fn desired_size(&self) -> (u32, u32) {
        let mut n = self.displayed().len() as f32;
        if n == 0.0 {
            return (self.config.width, 0);
        }
        if self.hidden_rows() > 0 {
            n += 1.0;
        }
        let scale = self.config.scale_f32();
        let avatar = self.config.avatar_size as f32 * scale;
        let spacing = self.config.spacing as f32 * scale;
        let row_h = avatar + RING_PADDING;
        let h = (n * row_h + (n - 1.0) * spacing) as u32;
        (self.config.width, h)
    }

    /// Compare the desired surface size with the stored one; if it changed,
    /// adopt the new size and return it for the shell to push as SizeChange.
    pub fn take_size_change(&mut self) -> Option<(u32, u32)> {
        let size = self.desired_size();
        if size != self.size && size.1 > 0 {
            self.size = size;
            return Some(size);
        }
        None
    }

    // -- avatars -------------------------------------------------------------

    pub fn avatar(&self, user_id: &str) -> Option<&Handle> {
        self.avatars.get(user_id)
    }

    pub fn insert_avatar(&mut self, user_id: String, data: Vec<u8>) {
        self.avatars.insert(user_id, data);
    }

    /// Mark every currently-missing avatar as in flight and return their
    /// (user_id, hash, url) triples for fetching.
    pub fn claim_missing_avatars(&mut self) -> Vec<(String, String, String)> {
        let missing = self.avatars.missing(&self.users);
        self.avatars
            .mark_requested(missing.iter().map(|(id, _, _)| id.clone()));
        missing
    }

    // -- placement ------------------------------------------------------------

    /// Recompute the surface margin from config (after an anchor change).
    pub fn reanchor(&mut self) {
        self.offset = super::geometry::offset(&self.config);
    }

    /// Apply a drag delta to the runtime margins.
    pub fn nudge(&mut self, dx: i32, dy: i32) {
        self.offset = super::geometry::drag(self.offset, &self.config, dx, dy);
    }

    /// Adopt the last-known roster from disk so a restart renders
    /// immediately; the RPC connection reconciles it within moments.
    pub fn hydrate_roster(&mut self) {
        if let Some(roster) = crate::daemon::adapters::cache::load_roster() {
            self.channel_name = roster.channel;
            self.me_id = roster.me_id;
            // Deserialization already restored the persisted fields; the
            // live-only speaking flag came back as false.
            self.users = roster.users;
            self.avatars.hydrate(&self.users);
        }
    }
}

#[cfg(test)]
mod tests {
    use hyprlay_core::config::Config;

    use super::*;

    fn participant(id: &str, name: &str, speaking: bool) -> Participant {
        Participant {
            id: id.to_string(),
            name: name.to_string(),
            avatar_hash: None,
            speaking,
            self_mute: false,
            self_deaf: false,
            server_mute: false,
            server_deaf: false,
        }
    }

    fn overlay(users: Vec<Participant>, cfg: Config) -> Overlay {
        let mut o = Overlay::new(cfg);
        o.users = users;
        o
    }

    fn ids<'a>(rows: &[&'a Participant]) -> Vec<&'a str> {
        rows.iter().map(|p| p.id.as_str()).collect()
    }

    #[test]
    fn join_order_keeps_wire_order() {
        let state = overlay(
            vec![
                participant("carol", "Carol", false),
                participant("alice", "alice", false),
                participant("bob", "Bob", false),
            ],
            Config::default(),
        );
        assert_eq!(ids(&state.displayed()), ["carol", "alice", "bob"]);
    }

    #[test]
    fn name_order_is_case_insensitive_a_to_z() {
        let cfg = Config {
            roster_order: RosterOrder::Name,
            ..Config::default()
        };
        let state = overlay(
            vec![
                participant("carol", "Carol", false),
                participant("dave", "dave", false),
                participant("alice", "ALICE", false),
                participant("bob", "Bob", false),
            ],
            cfg,
        );
        assert_eq!(ids(&state.displayed()), ["alice", "bob", "carol", "dave"]);
    }

    #[test]
    fn name_order_ties_fall_back_to_join_order() {
        let cfg = Config {
            roster_order: RosterOrder::Name,
            ..Config::default()
        };
        let state = overlay(
            vec![
                participant("second", "sam", false),
                participant("first", "Sam", false),
            ],
            cfg,
        );
        assert_eq!(ids(&state.displayed()), ["second", "first"]);
    }

    #[test]
    fn recent_speakers_bubble_up_most_recent_first() {
        let cfg = Config {
            roster_order: RosterOrder::RecentSpeakers,
            ..Config::default()
        };
        let all = || {
            vec![
                participant("a", "a", false),
                participant("b", "b", false),
                participant("c", "c", false),
            ]
        };
        let mut state = overlay(all(), cfg);
        // b starts, then a joins in: a is the most recent speaker.
        let mut b_speaking = all();
        b_speaking[1].speaking = true;
        state.apply_discord(DiscordEvent::Participants(b_speaking.clone()));
        let mut both = b_speaking.clone();
        both[0].speaking = true;
        state.apply_discord(DiscordEvent::Participants(both));
        assert_eq!(ids(&state.displayed()), ["a", "b", "c"]);
    }

    #[test]
    fn recent_speaker_sinks_back_when_stopping() {
        let cfg = Config {
            roster_order: RosterOrder::RecentSpeakers,
            ..Config::default()
        };
        let all = || vec![participant("b", "b", false), participant("a", "a", false)];
        let mut state = overlay(all(), cfg);
        let mut a_speaking = all();
        a_speaking[1].speaking = true;
        state.apply_discord(DiscordEvent::Participants(a_speaking));
        assert_eq!(ids(&state.displayed()), ["a", "b"]);
        // Stopping sinks a back to its join-order slot.
        state.apply_discord(DiscordEvent::Participants(all()));
        assert_eq!(ids(&state.displayed()), ["b", "a"]);
    }

    #[test]
    fn recent_speakers_cleared_on_channel_switch() {
        let cfg = Config {
            roster_order: RosterOrder::RecentSpeakers,
            ..Config::default()
        };
        let mut state = overlay(
            vec![participant("a", "a", false), participant("b", "b", false)],
            cfg,
        );
        state.apply_discord(DiscordEvent::Participants(vec![
            participant("a", "a", true),
            participant("b", "b", false),
        ]));
        assert_eq!(ids(&state.displayed()), ["a", "b"]);
        // Empty roster then a fresh list in reverse join order: the old
        // speaker record must not bubble `a` back to the top.
        state.apply_discord(DiscordEvent::Participants(vec![]));
        state.apply_discord(DiscordEvent::Participants(vec![
            participant("b", "b", false),
            participant("a", "a", false),
        ]));
        assert_eq!(ids(&state.displayed()), ["b", "a"]);
    }

    #[test]
    fn desired_size_grows_one_row_and_spacing_per_participant() {
        let cfg = Config {
            avatar_size: 34,
            spacing: 4,
            scale: 100,
            ..Config::default()
        };
        let users = vec![participant("1", "a", false), participant("2", "b", false)];
        let state = overlay(users, cfg.clone());
        // 2 rows: 2 * (34+8) + 1 * 4
        assert_eq!(state.desired_size(), (cfg.width, 88));
    }

    #[test]
    fn desired_size_scales_with_configured_scale() {
        let cfg = Config {
            avatar_size: 34,
            spacing: 4,
            scale: 200,
            ..Config::default()
        };
        let state = overlay(vec![participant("1", "a", false)], cfg.clone());
        // avatar 34*2 + unscaled 8px ring padding
        assert_eq!(state.desired_size(), (cfg.width, 76));
    }

    #[test]
    fn desired_size_is_zero_height_without_participants() {
        let state = overlay(vec![], Config::default());
        assert_eq!(state.desired_size().1, 0);
    }

    #[test]
    fn row_cap_truncates_and_reports_hidden_overflow() {
        let users = || {
            vec![
                participant("1", "a", false),
                participant("2", "b", false),
                participant("3", "c", false),
                participant("4", "d", false),
            ]
        };
        let cfg = Config {
            max_rows: 2,
            ..Config::default()
        };
        let state = overlay(users(), cfg);
        assert_eq!(ids(&state.displayed()), ["1", "2"]);
        assert_eq!(state.hidden_rows(), 2);
    }

    #[test]
    fn row_cap_zero_is_unlimited_and_no_pill_when_within_cap() {
        let users = || {
            vec![
                participant("1", "a", false),
                participant("2", "b", false),
                participant("3", "c", false),
            ]
        };
        let unlimited = overlay(users(), Config::default());
        assert_eq!(unlimited.displayed().len(), 3);
        assert_eq!(unlimited.hidden_rows(), 0);
        // n < cap
        let roomy = overlay(
            users(),
            Config {
                max_rows: 5,
                ..Config::default()
            },
        );
        assert_eq!(roomy.displayed().len(), 3);
        assert_eq!(roomy.hidden_rows(), 0);
        // n = cap
        let exact = overlay(
            users(),
            Config {
                max_rows: 3,
                ..Config::default()
            },
        );
        assert_eq!(exact.displayed().len(), 3);
        assert_eq!(exact.hidden_rows(), 0);
    }

    #[test]
    fn row_cap_counts_hidden_rows_after_filters() {
        // Talking-only hides the quiet rows before the cap applies, so the
        // pill counts only eligible rows that overflow the cap.
        let cfg = Config {
            show_only_talking_users: true,
            max_rows: 2,
            ..Config::default()
        };
        let state = overlay(
            vec![
                participant("1", "quiet", false),
                participant("2", "loud", true),
                participant("3", "loud", true),
                participant("4", "loud", true),
            ],
            cfg,
        );
        assert_eq!(ids(&state.displayed()), ["2", "3"]);
        assert_eq!(state.hidden_rows(), 1);
    }

    #[test]
    fn row_cap_truncates_after_sort() {
        let cfg = Config {
            roster_order: RosterOrder::Name,
            max_rows: 2,
            ..Config::default()
        };
        let state = overlay(
            vec![
                participant("z", "zed", false),
                participant("a", "amy", false),
                participant("m", "mo", false),
            ],
            cfg,
        );
        assert_eq!(ids(&state.displayed()), ["a", "m"]);
        assert_eq!(state.hidden_rows(), 1);
    }

    #[test]
    fn desired_size_adds_one_pill_row_when_truncated() {
        let users = || {
            vec![
                participant("1", "a", false),
                participant("2", "b", false),
                participant("3", "c", false),
            ]
        };
        let cfg = Config {
            avatar_size: 34,
            spacing: 4,
            scale: 100,
            ..Config::default()
        };
        // row_h = 34 + 8 = 42, spacing = 4.
        let full = overlay(users(), cfg.clone());
        let full_h = full.desired_size().1; // 3 rows: 3*42 + 2*4
        assert_eq!(full_h, 134);
        // Capped to 2 rows + a pill row = still 3 rows of height.
        let capped = overlay(
            users(),
            Config {
                max_rows: 2,
                ..cfg.clone()
            },
        );
        assert_eq!(capped.desired_size().1, full_h);
        assert_eq!(capped.hidden_rows(), 1);
    }

    #[test]
    fn displayed_hides_own_user_when_show_own_user_is_false() {
        let cfg = Config {
            show_own_user: false,
            ..Config::default()
        };
        let mut state = overlay(
            vec![
                participant("me", "me", false),
                participant("them", "them", false),
            ],
            cfg,
        );
        state.apply_discord(DiscordEvent::Me("me".to_string()));
        let shown = state.displayed();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].id, "them");
    }

    #[test]
    fn hidden_overlay_displays_no_rows_even_with_participants() {
        let cfg = Config {
            visible: false,
            ..Config::default()
        };
        let mut state = overlay(
            vec![participant("1", "a", true), participant("2", "b", false)],
            cfg,
        );
        assert!(state.displayed().is_empty());
        // Unhiding brings every filter-eligible participant straight back.
        state.config_mut().visible = true;
        assert_eq!(state.displayed().len(), 2);
    }

    #[test]
    fn displayed_keeps_only_speaking_users_when_talking_filter_is_set() {
        let cfg = Config {
            show_only_talking_users: true,
            ..Config::default()
        };
        let state = overlay(
            vec![
                participant("1", "quiet", false),
                participant("2", "loud", true),
            ],
            cfg,
        );
        let shown = state.displayed();
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].id, "2");
    }

    fn participant_with_avatar(id: &str, hash: &str) -> Participant {
        Participant {
            avatar_hash: Some(hash.to_string()),
            ..participant(id, id, false)
        }
    }

    #[test]
    fn claim_missing_avatars_lists_only_uncached_unrequested_users_once() {
        let mut state = overlay(
            vec![
                participant_with_avatar("1", "h1"),
                participant_with_avatar("2", "h2"),
                participant_with_avatar("3", "h3"),
                participant("4", "no-hash", false),
            ],
            Config::default(),
        );
        let first = state.claim_missing_avatars();
        assert_eq!(first.len(), 3);
        // Claiming is deduplicating: nothing is reported twice.
        assert!(state.claim_missing_avatars().is_empty());
    }

    #[test]
    fn insert_avatar_caches_bytes_for_rendering() {
        let mut state = overlay(vec![participant_with_avatar("9", "h9")], Config::default());
        // 1x1 transparent PNG
        let png: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9C, 0x62, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        state.insert_avatar("9".to_string(), png.to_vec());
        assert!(state.avatar("9").is_some());
    }

    #[test]
    fn participants_event_reports_changed_and_status_does_not() {
        let mut state = Overlay::new(Config::default());
        assert_eq!(
            state.apply_discord(DiscordEvent::Participants(vec![participant(
                "1", "a", false
            )])),
            RosterChange::Changed
        );
        assert_eq!(
            state.apply_discord(DiscordEvent::Status(ConnectionStatus::Connected)),
            RosterChange::Unchanged
        );
        assert_eq!(state.status(), ConnectionStatus::Connected);
    }

    #[test]
    fn take_size_change_adopts_new_size_once() {
        let mut state = overlay(vec![participant("1", "a", false)], Config::default());
        let first = state.take_size_change();
        assert!(first.is_some());
        assert_eq!(first.unwrap().1, state.size().1);
        // Same roster again → no change to report.
        assert!(state.take_size_change().is_none());
    }

    #[test]
    fn hovered_state_is_diff_gated_and_affects_alphas() {
        let mut state = Overlay::new(Config {
            opacity: 100,
            hover_opacity: 30,
            ..Config::default()
        });
        assert!(!state.is_hovered());
        assert_eq!(state.effective_alphas().overall, 1.0);
        assert!(state.set_hovered(true));
        assert!(state.is_hovered());
        assert_eq!(state.effective_alphas().overall, 0.3);
        assert!(!state.set_hovered(true));
        assert!(state.set_hovered(false));
        assert_eq!(state.effective_alphas().overall, 1.0);
    }

    #[test]
    fn effective_alphas_multiplies_per_part_with_hover_opacity() {
        let cfg = Config {
            opacity: 100,
            hover_opacity: 40,
            avatar_opacity: 100,
            text_opacity: 50,
            box_opacity: 90,
            ..Config::default()
        };
        let mut state = overlay(vec![participant("1", "a", false)], cfg);
        assert_eq!(state.effective_alphas().overall, 1.0);
        state.set_hovered(true);
        let a = state.effective_alphas();
        assert_eq!(a.overall, 0.4);
        assert_eq!(a.avatar, 0.4);
        assert_eq!(a.text, 0.2);
        assert_eq!(a.box_bg, 0.36);
    }
}
