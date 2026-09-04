//! The `status` reply line: one source of truth for the byte-stable wire
//! contract shared by the daemon (writer) and the tray/GUI (readers).
//!
//! The daemon fills a [`StatusFields`] from live state and config and sends
//! [`to_wire`](StatusFields::to_wire) as the `status` reply; every front
//! reads replies back through [`parse_wire`](StatusFields::parse_wire).
//! Field order and spelling are the observable contract (CONTEXT.md:
//! replies are pinned byte-for-byte; ADR-002: new fields append at the end),
//! so both directions live in this one module and change together.
//!
//! Parsing stays lenient bug-for-bug: an unreadable number yields 0, a
//! missing trailing field (older daemon) yields its default, and the
//! `channel` value is sliced between its markers because channel names may
//! contain spaces.

use std::fmt;

use crate::domain::ConnectionStatus;

/// One parsed `status=` reply, one field per wire key.
#[derive(Debug, PartialEq, Eq)]
pub struct StatusFields {
    /// Connection-status word, e.g. `connected`, `disconnected`, `off`.
    /// Writers build it from the sealed [`ConnectionStatus`] vocabulary;
    /// readers get the raw wire token, so a word this build does not know
    /// still parses (forward-compat) and reaches the UI verbatim.
    pub status_word: StatusWord,
    /// Voice-channel name; `-` when not connected. May contain spaces.
    pub channel: String,
    /// Number of roster rows the overlay displays.
    pub participants: usize,
    /// Corner word (`top-left` … `bottom-right`).
    pub position: String,
    /// Right-to-left layout flag.
    pub rtl: bool,
    /// Overlay visibility flag.
    pub visible: bool,
    /// Anchor-mode word (`auto`, `top`, `bottom`).
    pub anchor: String,
    /// Global scale in percent.
    pub scale: u8,
    /// Overall opacity in percent.
    pub opacity: u8,
    /// Horizontal distance in px from the anchored screen edge.
    pub offset_x: i32,
    /// Vertical distance in px from the anchored screen edge.
    pub offset_y: i32,
    /// Target output name; `None` renders as `active`, and parsing maps
    /// the literal `active` back to `None` (the wire cannot tell them apart).
    pub monitor: Option<String>,
    /// Auth backend label, e.g. `own-app`.
    pub auth: String,
    /// Overlay-layer flag from ADR-002.
    pub show_on_fullscreen: bool,
    /// Hover-dim flag from ADR-002.
    pub dim_on_hover: bool,
    /// Hover opacity in percent from ADR-002.
    pub hover_opacity: u8,
}

impl StatusFields {
    /// Whether a reply line is a status reply. The cheap pre-filter for
    /// liveness probes that only need "a daemon answered `status`".
    pub fn is_status_line(reply: &str) -> bool {
        reply.starts_with("status=")
    }

    /// Render the wire line, byte-identical to the daemon's `status` reply.
    pub fn to_wire(&self) -> String {
        format!(
            "status={} channel={} participants={} position={} rtl={} visible={} \
             anchor={} scale={} opacity={} offset=({},{}) monitor={} auth={} \
             show-on-fullscreen={} dim-on-hover={} hover-opacity={}",
            self.status_word,
            self.channel,
            self.participants,
            self.position,
            on_off(self.rtl),
            on_off(self.visible),
            self.anchor,
            self.scale,
            self.opacity,
            self.offset_x,
            self.offset_y,
            self.monitor.as_deref().unwrap_or("active"),
            self.auth,
            on_off(self.show_on_fullscreen),
            on_off(self.dim_on_hover),
            self.hover_opacity,
        )
    }

    /// Parse a `status` reply line. Returns `None` for non-status lines.
    /// Lenient bug-for-bug: an unreadable number yields 0 and a missing
    /// field (older daemon) yields its default.
    pub fn parse_wire(line: &str) -> Option<Self> {
        if !Self::is_status_line(line) {
            return None;
        }
        // Channel names may contain spaces, so the value is sliced between
        // its markers instead of read as a token.
        let channel = line
            .find("channel=")
            .and_then(|start| {
                line[start..]
                    .find(" participants=")
                    .map(|end| line[start + "channel=".len()..start + end].to_string())
            })
            .unwrap_or_default();
        let (offset_x, offset_y) = parse_offset(token_after(line, " offset="));
        Some(Self {
            status_word: StatusWord(word(line, "status=")),
            channel,
            participants: number(token_after(line, " participants=")),
            position: word(line, " position="),
            rtl: token_after(line, " rtl=") == Some("on"),
            visible: token_after(line, " visible=") == Some("on"),
            anchor: word(line, " anchor="),
            scale: number(token_after(line, " scale=")),
            opacity: number(token_after(line, " opacity=")),
            offset_x,
            offset_y,
            monitor: match token_after(line, " monitor=") {
                Some("active") | None => None,
                Some(name) => Some(name.to_string()),
            },
            auth: word(line, " auth="),
            show_on_fullscreen: token_after(line, " show-on-fullscreen=") == Some("on"),
            dim_on_hover: token_after(line, " dim-on-hover=") == Some("on"),
            hover_opacity: number(token_after(line, " hover-opacity=")),
        })
    }
}

/// The connection-status word of a `status` reply. Opaque on purpose: the
/// wire contract has a sealed write side and a lenient read side.
///
/// - **Write** (daemon): the only way in is [`From<ConnectionStatus>`], so a
///   bare string can never reach the wire and the word set cannot drift from
///   the enum's `Display` spelling.
/// - **Read** (tray/GUI): [`StatusFields::parse_wire`] fills the word from
///   the raw line, unknown words included — a newer daemon may already speak
///   words this build has never seen (ADR-002 forward-compat).
///
/// [`Display`] renders the stored word unchanged in both directions, which
/// is what keeps `to_wire()` byte-identical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusWord(String);

impl From<ConnectionStatus> for StatusWord {
    fn from(status: ConnectionStatus) -> Self {
        Self(status.to_string())
    }
}

impl fmt::Display for StatusWord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl StatusWord {
    /// The raw wire token. Readers compare or show it verbatim; nothing
    /// maps it back onto [`ConnectionStatus`], because the vocabulary is
    /// open on the wire (a future daemon's word must survive the round trip).
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

/// The whitespace-delimited text that follows `marker` in the line, without
/// the marker. Field markers carry their leading space so they can only
/// match at a field boundary; `status=` is the exception, anchored at
/// position 0 by [`StatusFields::is_status_line`].
fn token_after<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let start = line.find(marker)? + marker.len();
    let rest = &line[start..];
    Some(&rest[..rest.find(' ').unwrap_or(rest.len())])
}

fn word(line: &str, marker: &str) -> String {
    token_after(line, marker).unwrap_or_default().to_string()
}

/// Lenient number reading, bug-for-bug: anything unparseable is 0.
fn number<T: std::str::FromStr + Default>(value: Option<&str>) -> T {
    value.and_then(|v| v.parse().ok()).unwrap_or_default()
}

/// The `offset=(x,y)` tuple form; malformed or missing yields `(0, 0)`.
fn parse_offset(value: Option<&str>) -> (i32, i32) {
    let inner = value
        .unwrap_or_default()
        .trim_start_matches('(')
        .trim_end_matches(')');
    let (x, y) = inner.split_once(',').unwrap_or_default();
    (number(Some(x)), number(Some(y)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_preserves_every_field() {
        let fields = StatusFields {
            status_word: ConnectionStatus::Connected.into(),
            channel: "ngobrol 3".into(),
            participants: 2,
            position: "bottom-right".into(),
            rtl: true,
            visible: true,
            anchor: "top".into(),
            scale: 120,
            opacity: 90,
            offset_x: -12,
            offset_y: 34,
            monitor: Some("eDP-1".into()),
            auth: "own-app".into(),
            show_on_fullscreen: true,
            dim_on_hover: true,
            hover_opacity: 40,
        };
        let parsed = StatusFields::parse_wire(&fields.to_wire()).expect("roundtrip parses");
        assert_eq!(parsed.status_word.as_str(), "connected");
        assert_eq!(parsed.channel, "ngobrol 3");
        assert_eq!(parsed.participants, 2);
        assert_eq!(parsed.position, "bottom-right");
        assert!(parsed.rtl);
        assert!(parsed.visible);
        assert_eq!(parsed.anchor, "top");
        assert_eq!(parsed.scale, 120);
        assert_eq!(parsed.opacity, 90);
        assert_eq!(parsed.offset_x, -12);
        assert_eq!(parsed.offset_y, 34);
        assert_eq!(parsed.monitor, Some("eDP-1".into()));
        assert_eq!(parsed.auth, "own-app");
        assert!(parsed.show_on_fullscreen);
        assert!(parsed.dim_on_hover);
        assert_eq!(parsed.hover_opacity, 40);
    }

    #[test]
    fn to_wire_matches_the_pinned_wire_line() {
        let fields = StatusFields {
            status_word: ConnectionStatus::Connected.into(),
            channel: "ngobrol 3".into(),
            participants: 2,
            position: "top-left".into(),
            rtl: true,
            visible: false,
            anchor: "auto".into(),
            scale: 100,
            opacity: 90,
            offset_x: -12,
            offset_y: 34,
            monitor: None,
            auth: "own-app".into(),
            show_on_fullscreen: true,
            dim_on_hover: false,
            hover_opacity: 40,
        };
        assert_eq!(
            fields.to_wire(),
            "status=connected channel=ngobrol 3 participants=2 position=top-left \
             rtl=on visible=off anchor=auto scale=100 opacity=90 offset=(-12,34) \
             monitor=active auth=own-app show-on-fullscreen=on dim-on-hover=off \
             hover-opacity=40"
        );
    }

    #[test]
    fn parse_wire_rejects_non_status_replies() {
        assert_eq!(StatusFields::parse_wire("error: daemon unreachable"), None);
        assert_eq!(StatusFields::parse_wire(""), None);
        assert_eq!(StatusFields::parse_wire("connecting…"), None);
    }

    #[test]
    fn parse_wire_reads_a_lenient_participant_count_as_zero() {
        // New pin, not a preserved behavior: the old tray echoed the raw
        // token ("abc") into the summary. The daemon only ever writes
        // numeric counts; lenient normalization to 0 is the ticket's spec.
        let fields =
            StatusFields::parse_wire("status=connected channel=a participants=abc").unwrap();
        assert_eq!(fields.participants, 0);
    }

    #[test]
    fn parse_wire_reads_a_malformed_offset_as_zero() {
        // Untested-branch pin: the (0, 0) fallback for a malformed tuple.
        let fields = StatusFields::parse_wire("status=connected channel=a offset=garbage").unwrap();
        assert_eq!(fields.offset_x, 0);
        assert_eq!(fields.offset_y, 0);
    }

    #[test]
    fn parse_wire_defaults_missing_trailing_fields() {
        // The oldest line shape: everything before ADR-002 appended its
        // three fields at the end.
        let fields = StatusFields::parse_wire(
            "status=connected channel=a participants=2 position=top-left rtl=off \
             visible=on anchor=auto scale=100 opacity=90 offset=(0,0) \
             monitor=active auth=own-app",
        )
        .unwrap();
        assert!(fields.visible);
        assert_eq!(fields.monitor, None);
        assert!(!fields.show_on_fullscreen);
        assert!(!fields.dim_on_hover);
        assert_eq!(fields.hover_opacity, 0);
    }

    #[test]
    fn parse_wire_ignores_unknown_trailing_fields() {
        // ADR-002's append-at-end convention made structural: a future
        // daemon's extra field must not corrupt the fields before it.
        let line = "status=connected channel=ngobrol 3 participants=2 position=top-left \
                    rtl=on visible=off anchor=auto scale=100 opacity=90 offset=(-12,34) \
                    monitor=active auth=own-app show-on-fullscreen=on dim-on-hover=off \
                    hover-opacity=40 future-knob=7";
        let fields = StatusFields::parse_wire(line).unwrap();
        assert_eq!(
            fields,
            StatusFields {
                status_word: ConnectionStatus::Connected.into(),
                channel: "ngobrol 3".into(),
                participants: 2,
                position: "top-left".into(),
                rtl: true,
                visible: false,
                anchor: "auto".into(),
                scale: 100,
                opacity: 90,
                offset_x: -12,
                offset_y: 34,
                monitor: None,
                auth: "own-app".into(),
                show_on_fullscreen: true,
                dim_on_hover: false,
                hover_opacity: 40,
            }
        );
    }

    #[test]
    fn parse_wire_reads_the_status_word_as_the_first_token() {
        // `exchanging token` is the only multi-word status; both current
        // readers take the first whitespace-delimited token.
        let fields =
            StatusFields::parse_wire("status=exchanging token channel=a participants=1").unwrap();
        assert_eq!(fields.status_word.as_str(), "exchanging");
        assert_eq!(fields.channel, "a");
        assert_eq!(fields.participants, 1);
    }

    #[test]
    fn parse_wire_keeps_an_unknown_status_word_verbatim() {
        // The reader-side half of the StatusWord contract: a future daemon
        // may speak a word this build has never seen (ADR-002 forward-compat).
        // The word must survive parsing and to_wire() byte-for-byte.
        let fields = StatusFields::parse_wire("status=relinking channel=a participants=1").unwrap();
        assert_eq!(fields.status_word.as_str(), "relinking");
        assert_eq!(
            fields.to_wire(),
            "status=relinking channel=a participants=1 position= rtl=off visible=off \
             anchor= scale=0 opacity=0 offset=(0,0) monitor=active auth= \
             show-on-fullscreen=off dim-on-hover=off hover-opacity=0"
        );
    }

    #[test]
    fn status_word_accepts_only_the_sealed_vocabulary_on_the_write_side() {
        // The writer-side half: the daemon's word set is the enum, spelled
        // exactly as its Display does (wire contract).
        let cases = [
            (ConnectionStatus::Connecting, "connecting"),
            (ConnectionStatus::Authorize, "authorize"),
            (ConnectionStatus::Authenticating, "authenticating"),
            (ConnectionStatus::ExchangingToken, "exchanging token"),
            (ConnectionStatus::Connected, "connected"),
            (ConnectionStatus::Disconnected, "disconnected"),
        ];
        for (status, word) in cases {
            let wire = StatusFields {
                status_word: status.into(),
                channel: "-".into(),
                participants: 0,
                position: "top-left".into(),
                rtl: false,
                visible: false,
                anchor: "auto".into(),
                scale: 100,
                opacity: 100,
                offset_x: 0,
                offset_y: 0,
                monitor: None,
                auth: "own-app".into(),
                show_on_fullscreen: true,
                dim_on_hover: false,
                hover_opacity: 40,
            }
            .to_wire();
            assert!(wire.starts_with(&format!("status={word} ")), "{wire}");
        }
    }
}
