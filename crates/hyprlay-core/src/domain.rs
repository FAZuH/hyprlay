//! Domain vocabulary shared by every layer: connection status, validated
//! colors, and the control-command language spoken over the ctl socket.
//!
//! A [`Command`] parses from the exact line format the CLI/GUI send and
//! displays back to that canonical form, so the wire protocol is unchanged.
//! Config mutations run through [`Command::apply_config`], which owns every
//! mutation rule (ranges, cross-field constraints, reply text) in one place.

use std::fmt;
use std::str::FromStr;

use serde::de::Deserializer;
use serde::de::Visitor;
use serde::ser::Serializer;

use crate::config::AVATAR_SIZE;
use crate::config::Bounds;
use crate::config::Config;
use crate::config::HorizontalAnchor;
use crate::config::MAX_NAME;
use crate::config::OFFSETS;
use crate::config::OPACITY;
use crate::config::SCALE;
use crate::config::SPACING;
use crate::config::TEXT_SIZE;
use crate::config::VerticalAnchor;
use crate::config::WIDTH;
use crate::config::{self};

// ---------------------------------------------------------------------------
// Connection status
// ---------------------------------------------------------------------------

/// Where the Discord RPC connection stands. The `Display` strings are part
/// of the observable surface (the `status` ctl reply and the overlay UI),
/// so they never change spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnectionStatus {
    #[default]
    Connecting,
    /// Waiting for the user to approve the OAuth prompt in Discord.
    Authorize,
    Authenticating,
    ExchangingToken,
    Connected,
    Disconnected,
}

impl ConnectionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Authorize => "authorize",
            Self::Authenticating => "authenticating",
            Self::ExchangingToken => "exchanging token",
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
        }
    }
}

impl fmt::Display for ConnectionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Colors
// ---------------------------------------------------------------------------

/// An `#rrggbb` (or `#rgb`) color, validated at the boundary so an invalid
/// value can never reach rendering or persistence.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HexColor([u8; 3]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseHexError;

impl fmt::Display for ParseHexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid color: expected #rrggbb")
    }
}

impl std::error::Error for ParseHexError {}

impl HexColor {
    pub const fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        Self([r, g, b])
    }

    pub const fn rgb(self) -> [u8; 3] {
        self.0
    }
}

impl FromStr for HexColor {
    type Err = ParseHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s.strip_prefix('#').ok_or(ParseHexError)?;
        let byte = |h: &str| u8::from_str_radix(h, 16).map_err(|_| ParseHexError);
        match hex.len() {
            3 => {
                let dup = |c: char| {
                    let mut s = String::with_capacity(2);
                    s.push(c);
                    s.push(c);
                    s
                };
                let r = byte(&dup(hex.chars().next().ok_or(ParseHexError)?))?;
                let g = byte(&dup(hex.chars().nth(1).ok_or(ParseHexError)?))?;
                let b = byte(&dup(hex.chars().nth(2).ok_or(ParseHexError)?))?;
                Ok(Self([r, g, b]))
            }
            6 => Ok(Self([
                byte(&hex[0..2])?,
                byte(&hex[2..4])?,
                byte(&hex[4..6])?,
            ])),
            _ => Err(ParseHexError),
        }
    }
}

impl fmt::Display for HexColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.0[0], self.0[1], self.0[2])
    }
}

impl serde::Serialize for HexColor {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for HexColor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = HexColor;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a #rrggbb color string")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<HexColor, E> {
                v.parse().map_err(|_| E::custom(ParseHexError))
            }
        }
        deserializer.deserialize_str(V)
    }
}

// ---------------------------------------------------------------------------
// Effects and command results
// ---------------------------------------------------------------------------

/// What the shell must do after a command mutates the config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Re-compute the surface size from the new config.
    Resize,
    /// Re-anchor the surface and reset margins from config.
    Reanchor,
    /// Move the surface by a pixel delta (runtime margins).
    Nudge(i32, i32),
}

#[derive(Debug)]
pub struct CommandResult {
    pub reply: String,
    pub effects: Vec<Effect>,
}

impl CommandResult {
    fn ok(reply: impl Into<String>, effects: Vec<Effect>) -> Self {
        Self {
            reply: reply.into(),
            effects,
        }
    }

    fn err(reply: impl Into<String>) -> Self {
        Self {
            reply: reply.into(),
            effects: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Daemon-side persistence policy for one applied command: with autosave
/// on, every state mutation is written through immediately; read-only and
/// lifecycle commands never touch disk; `save` is the explicit force-write
/// that persists even while the switch is off. Pure — the caller still has
/// to skip persisting when the command itself failed.
pub fn should_persist(cmd: &Command, auto_save: bool) -> bool {
    match cmd {
        Command::Save => true,
        Command::Status
        | Command::Help
        | Command::Dump
        | Command::Get(_)
        | Command::Reload
        | Command::Restart
        | Command::Quit
        // Runtime-only placement: nudge shifts the surface without changing
        // any persisted key, so writing config.toml would be a no-op save.
        | Command::Nudge(_, _) => false,
        _ => auto_save,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonitorTarget {
    Active,
    Named(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// One screen edge for the `move` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Center,
    Top,
    Bottom,
}

/// One config key: the vocabulary shared by `get`/`set` on the wire, the
/// GUI field registry, and the revert diff. Every key knows its group, its
/// bounds, how to read and write itself, and what the shell must re-do
/// after a change — so no surface can drift from another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    // position group
    Position,
    Anchor,
    Monitor,
    OffsetX,
    OffsetY,
    OffsetMin,
    OffsetMax,
    Rtl,
    // layout group
    Width,
    Scale,
    AvatarSize,
    TextSize,
    Spacing,
    MaxName,
    TalkingOnly,
    OwnUser,
    // opacity group
    Opacity,
    AvatarOpacity,
    TextOpacity,
    BoxOpacity,
    // colors group
    SpeakingColor,
    TextColor,
    BoxColor,
    // layout group, but appended last: the wire grammar is byte-stable for
    // existing keys and new keys join the end of the shared table.
    Visible,
    AutoSave,
    ShowOnFullscreen,
    DimOnHover,
    HoverOpacity,
}

/// Config sections, shared by `reset <group>` and the TOML layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Position,
    Layout,
    Opacity,
    Colors,
}

impl Group {
    pub const ALL: [Group; 4] = [
        Group::Position,
        Group::Layout,
        Group::Opacity,
        Group::Colors,
    ];

    fn parse(word: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|group| group.to_string() == word)
    }
}

impl fmt::Display for Group {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Position => "position",
            Self::Layout => "layout",
            Self::Opacity => "opacity",
            Self::Colors => "colors",
        })
    }
}

/// A typed value for one key. `Cycle` is the bare `set <key>` form: enum
/// keys advance to their next option, flag keys flip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Num(i64),
    Flag(bool),
    Color(HexColor),
    Corner(Corner),
    Anchor(crate::config::AnchorMode),
    Target(MonitorTarget),
    Cycle,
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Num(v) => write!(f, "{v}"),
            Self::Flag(v) => write!(f, "{}", on_off(*v)),
            Self::Color(c) => write!(f, "{c}"),
            Self::Corner(c) => f.write_str(corner_word(*c)),
            Self::Target(MonitorTarget::Active) => f.write_str("active"),
            Self::Anchor(m) => f.write_str(m.as_str()),
            Self::Target(MonitorTarget::Named(name)) => write!(f, "{name}"),
            // Never sent over the wire; only ever constructed internally.
            Self::Cycle => f.write_str(""),
        }
    }
}

impl Key {
    /// Every key in display order (grouped, wire order inside a group).
    pub const ALL: [Key; 28] = [
        Key::Position,
        Key::Anchor,
        Key::Monitor,
        Key::OffsetX,
        Key::OffsetY,
        Key::OffsetMin,
        Key::OffsetMax,
        Key::Rtl,
        Key::Width,
        Key::Scale,
        Key::AvatarSize,
        Key::TextSize,
        Key::Spacing,
        Key::MaxName,
        Key::TalkingOnly,
        Key::OwnUser,
        Key::Opacity,
        Key::AvatarOpacity,
        Key::TextOpacity,
        Key::BoxOpacity,
        Key::SpeakingColor,
        Key::TextColor,
        Key::BoxColor,
        Key::Visible,
        Key::AutoSave,
        Key::ShowOnFullscreen,
        Key::DimOnHover,
        Key::HoverOpacity,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Position => "position",
            Self::Anchor => "anchor",
            Self::Monitor => "monitor",
            Self::OffsetX => "offset-x",
            Self::OffsetY => "offset-y",
            Self::OffsetMin => "offset-min",
            Self::OffsetMax => "offset-max",
            Self::Rtl => "rtl",
            Self::Width => "width",
            Self::Scale => "scale",
            Self::AvatarSize => "avatar-size",
            Self::TextSize => "text-size",
            Self::Spacing => "spacing",
            Self::MaxName => "max-name",
            Self::TalkingOnly => "talking-only",
            Self::OwnUser => "own-user",
            Self::Opacity => "opacity",
            Self::AvatarOpacity => "avatar-opacity",
            Self::TextOpacity => "text-opacity",
            Self::BoxOpacity => "box-opacity",
            Self::SpeakingColor => "speaking-color",
            Self::TextColor => "text-color",
            Self::BoxColor => "box-color",
            Self::Visible => "visible",
            Self::AutoSave => "auto-save",
            Self::ShowOnFullscreen => "show-on-fullscreen",
            Self::DimOnHover => "dim-on-hover",
            Self::HoverOpacity => "hover-opacity",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.name() == name)
    }

    pub fn group(self) -> Group {
        match self {
            Self::Position
            | Self::Anchor
            | Self::Monitor
            | Self::OffsetX
            | Self::OffsetY
            | Self::OffsetMin
            | Self::OffsetMax
            | Self::Rtl => Group::Position,
            Self::Width
            | Self::Scale
            | Self::AvatarSize
            | Self::TextSize
            | Self::Spacing
            | Self::MaxName
            | Self::TalkingOnly
            | Self::OwnUser
            | Self::Visible
            | Self::AutoSave
            | Self::ShowOnFullscreen
            | Self::DimOnHover => Group::Layout,
            Self::Opacity
            | Self::AvatarOpacity
            | Self::TextOpacity
            | Self::BoxOpacity
            | Self::HoverOpacity => Group::Opacity,
            Self::SpeakingColor | Self::TextColor | Self::BoxColor => Group::Colors,
        }
    }

    /// Canonical `key=value` reply text for the current state.
    pub fn get(self, cfg: &Config) -> String {
        let value = self.value_of(cfg);
        format!("{}={value}", self.name())
    }

    /// The current value as a typed [`Value`], ready to feed back into
    /// [`Key::apply`] or [`Command::Set`].
    pub fn value_of(self, cfg: &Config) -> Value {
        match self {
            Self::Position => Value::Corner(corner_of(cfg.horizontal, cfg.vertical)),
            Self::Anchor => Value::Anchor(cfg.anchor),
            Self::Monitor => Value::Target(match &cfg.monitor {
                None => MonitorTarget::Active,
                Some(name) => MonitorTarget::Named(name.clone()),
            }),
            Self::OffsetX => Value::Num(cfg.offset_x as i64),
            Self::OffsetY => Value::Num(cfg.offset_y as i64),
            Self::OffsetMin => Value::Num(cfg.offset_min as i64),
            Self::OffsetMax => Value::Num(cfg.offset_max as i64),
            Self::Rtl => Value::Flag(cfg.rtl),
            Self::Width => Value::Num(cfg.width as i64),
            Self::Scale => Value::Num(cfg.scale as i64),
            Self::AvatarSize => Value::Num(cfg.avatar_size as i64),
            Self::TextSize => Value::Num(cfg.text_size as i64),
            Self::Spacing => Value::Num(cfg.spacing as i64),
            Self::MaxName => Value::Num(cfg.max_username_length as i64),
            Self::TalkingOnly => Value::Flag(cfg.show_only_talking_users),
            Self::OwnUser => Value::Flag(cfg.show_own_user),
            Self::Visible => Value::Flag(cfg.visible),
            Self::AutoSave => Value::Flag(cfg.auto_save),
            Self::ShowOnFullscreen => Value::Flag(cfg.show_on_fullscreen),
            Self::DimOnHover => Value::Flag(cfg.dim_on_hover),
            Self::Opacity => Value::Num(cfg.opacity as i64),
            Self::AvatarOpacity => Value::Num(cfg.avatar_opacity as i64),
            Self::TextOpacity => Value::Num(cfg.text_opacity as i64),
            Self::BoxOpacity => Value::Num(cfg.box_opacity as i64),
            Self::HoverOpacity => Value::Num(cfg.hover_opacity as i64),
            Self::SpeakingColor => Value::Color(cfg.speaking_color),
            Self::TextColor => Value::Color(cfg.text_color),
            Self::BoxColor => Value::Color(cfg.box_color),
        }
    }

    /// Hard validation bounds for numeric keys; `None` for non-numeric ones.
    pub fn num_bounds(self) -> Option<(i64, i64)> {
        let pair = |b: Bounds<i32>| (b.min as i64, b.max as i64);
        match self {
            Self::OffsetX | Self::OffsetY | Self::OffsetMin | Self::OffsetMax => {
                Some(pair(OFFSETS))
            }
            Self::Opacity
            | Self::AvatarOpacity
            | Self::TextOpacity
            | Self::BoxOpacity
            | Self::HoverOpacity => Some((OPACITY.min as i64, OPACITY.max as i64)),
            Self::Width => Some((WIDTH.min as i64, WIDTH.max as i64)),
            Self::Scale => Some((SCALE.min as i64, SCALE.max as i64)),
            Self::AvatarSize => Some((AVATAR_SIZE.min as i64, AVATAR_SIZE.max as i64)),
            Self::TextSize => Some((TEXT_SIZE.min as i64, TEXT_SIZE.max as i64)),
            Self::Spacing => Some((SPACING.min as i64, SPACING.max as i64)),
            Self::MaxName => Some((MAX_NAME.min as i64, MAX_NAME.max as i64)),
            _ => None,
        }
    }

    /// Slider envelope for the GUI; `None` renders an integer input without
    /// a slider. Offset x/y follow the user-configured min/max window.
    pub fn slider_bounds(self, cfg: &Config) -> Option<(f32, f32)> {
        match self {
            Self::OffsetMin | Self::OffsetMax => None,
            Self::OffsetX | Self::OffsetY => Some((cfg.offset_min as f32, cfg.offset_max as f32)),
            other => other
                .num_bounds()
                .map(|(min, max)| (min as f32, max as f32)),
        }
    }
}

pub fn corner_word(corner: Corner) -> &'static str {
    match corner {
        Corner::TopLeft => "top-left",
        Corner::TopRight => "top-right",
        Corner::BottomLeft => "bottom-left",
        Corner::BottomRight => "bottom-right",
    }
}

fn corner_parse(word: &str) -> Option<Corner> {
    Some(match word {
        "top-left" | "tl" => Corner::TopLeft,
        "top-right" | "tr" => Corner::TopRight,
        "bottom-left" | "bl" => Corner::BottomLeft,
        "bottom-right" | "br" => Corner::BottomRight,
        _ => return None,
    })
}

const CORNER_CYCLE: [Corner; 4] = [
    Corner::TopLeft,
    Corner::TopRight,
    Corner::BottomLeft,
    Corner::BottomRight,
];

/// Keys whose bare `set <key>` form advances to the next option instead of
/// requiring a value: flags flip, enums step through their choices.
fn cycle_able(key: Key) -> bool {
    matches!(
        key,
        Key::Position
            | Key::Anchor
            | Key::Monitor
            | Key::Rtl
            | Key::TalkingOnly
            | Key::OwnUser
            | Key::Visible
            | Key::AutoSave
            | Key::ShowOnFullscreen
            | Key::DimOnHover
    )
}

/// The `<...>` fragment for parse errors, from the shared bounds table.
fn hint_for(key: Key) -> String {
    match key.num_bounds() {
        Some((min, max)) => format!("<{min}-{max}>"),
        None if matches!(key, Key::SpeakingColor | Key::TextColor | Key::BoxColor) => {
            "<#rrggbb>".to_string()
        }
        None => String::new(),
    }
}

impl Key {
    /// Parse one raw wire token into a typed value under this key's rules.
    /// A missing token yields [`Value::Cycle`] for cycle-able keys and an
    /// error for keys that require a value.
    pub fn parse_value(self, arg: Option<&str>) -> Result<Value, String> {
        // Bare `set <key>` on a cycle-able key means "advance to the next
        // option" / "flip"; every other key requires an explicit value.
        if arg.is_none() && cycle_able(self) {
            return Ok(Value::Cycle);
        }
        let flag = |name: &str| -> Result<Value, String> {
            match arg {
                None => Ok(Value::Cycle),
                Some("on") | Some("true") | Some("1") => Ok(Value::Flag(true)),
                Some("off") | Some("false") | Some("0") => Ok(Value::Flag(false)),
                _ => Err(format!("error: {name} <on|off>")),
            }
        };
        let num = || -> Result<Value, String> {
            let (min, max) = self.num_bounds().expect("numeric key");
            match arg.and_then(|v| v.parse::<i64>().ok()) {
                Some(v) if v >= min && v <= max => Ok(Value::Num(v)),
                _ => Err(format!("error: {} {}", self.name(), hint_for(self))),
            }
        };
        let color = |name: &str| -> Result<Value, String> {
            arg.and_then(|c| c.parse().ok())
                .map(Value::Color)
                .ok_or_else(|| format!("error: {name} <#rrggbb>"))
        };
        match self {
            Self::Position => corner_parse(arg.unwrap_or_default())
                .map(Value::Corner)
                .ok_or_else(|| {
                    "error: position <top-left|top-right|bottom-left|bottom-right>".to_string()
                }),
            Self::Anchor => match arg {
                Some("auto") => Ok(Value::Anchor(crate::config::AnchorMode::Auto)),
                Some("top") => Ok(Value::Anchor(crate::config::AnchorMode::Top)),
                Some("bottom") => Ok(Value::Anchor(crate::config::AnchorMode::Bottom)),
                _ => Err("error: anchor <auto|top|bottom>".to_string()),
            },
            Self::Monitor => Ok(match arg {
                Some("active") => Value::Target(MonitorTarget::Active),
                Some(name) => Value::Target(MonitorTarget::Named(name.to_string())),
                None => unreachable!("monitor cycles before this match"),
            }),
            Self::Rtl => flag(self.name()),
            Self::TalkingOnly => flag(self.name()),
            Self::OwnUser => flag(self.name()),
            Self::Visible => flag(self.name()),
            Self::AutoSave => flag(self.name()),
            Self::ShowOnFullscreen => flag(self.name()),
            Self::DimOnHover => flag(self.name()),
            Self::OffsetX
            | Self::OffsetY
            | Self::OffsetMin
            | Self::OffsetMax
            | Self::Width
            | Self::Scale
            | Self::AvatarSize
            | Self::TextSize
            | Self::Spacing
            | Self::MaxName
            | Self::Opacity
            | Self::AvatarOpacity
            | Self::TextOpacity
            | Self::BoxOpacity
            | Self::HoverOpacity => num(),
            Self::SpeakingColor => color(self.name()),
            Self::TextColor => color(self.name()),
            Self::BoxColor => color(self.name()),
        }
    }

    /// Mutate the config under this key's rules. Cycle resolution happens
    /// here because it depends on the current value. The reply is always the
    /// canonical `key=value` text (the same string `get` would return).
    pub fn apply(self, cfg: &mut Config, value: Value) -> CommandResult {
        let value = match value {
            Value::Cycle => match self {
                Self::Position => Value::Corner(corner_of(cfg.horizontal, cfg.vertical).next()),
                Self::Anchor => Value::Anchor(cfg.anchor.next()),
                Self::Monitor => {
                    return CommandResult::err("error: monitor cycling needs the running daemon");
                }
                Self::Rtl => Value::Flag(!cfg.rtl),
                Self::TalkingOnly => Value::Flag(!cfg.show_only_talking_users),
                Self::OwnUser => Value::Flag(!cfg.show_own_user),
                Self::Visible => Value::Flag(!cfg.visible),
                Self::AutoSave => Value::Flag(!cfg.auto_save),
                Self::DimOnHover => Value::Flag(!cfg.dim_on_hover),
                Self::ShowOnFullscreen => {
                    return CommandResult::err("error: not a config command");
                }
                _ => return CommandResult::err(format!("error: {} requires a value", self.name())),
            },
            other => other,
        };
        match (self, value) {
            (Self::Position, Value::Corner(corner)) => {
                let (horizontal, vertical) = match corner {
                    Corner::TopLeft => (HorizontalAnchor::Left, VerticalAnchor::Top),
                    Corner::TopRight => (HorizontalAnchor::Right, VerticalAnchor::Top),
                    Corner::BottomLeft => (HorizontalAnchor::Left, VerticalAnchor::Bottom),
                    Corner::BottomRight => (HorizontalAnchor::Right, VerticalAnchor::Bottom),
                };
                cfg.horizontal = horizontal;
                cfg.vertical = vertical;
                // Right-side presets flip to RTL so names hug the screen edge.
                cfg.rtl = horizontal == HorizontalAnchor::Right;
                CommandResult::ok(
                    format!("position={} rtl={}", corner_word(corner), on_off(cfg.rtl)),
                    vec![Effect::Reanchor, Effect::Resize],
                )
            }
            (Self::Anchor, Value::Anchor(mode)) => {
                cfg.anchor = mode;
                CommandResult::ok(format!("anchor={mode}"), vec![Effect::Reanchor])
            }
            // Routed by the daemon shell before config application: a change
            // re-creates the layer surface on another output.
            (Self::Monitor, _) => CommandResult::err("error: not a config command"),
            (Self::ShowOnFullscreen, _) => CommandResult::err("error: not a config command"),
            (Self::DimOnHover, Value::Flag(v)) => {
                set_flag(&mut cfg.dim_on_hover, v, "dim-on-hover", Effect::Resize)
            }
            (Self::Rtl, Value::Flag(v)) => set_flag(&mut cfg.rtl, v, "rtl", Effect::Resize),
            (Self::TalkingOnly, Value::Flag(v)) => set_flag(
                &mut cfg.show_only_talking_users,
                v,
                "talking-only",
                Effect::Resize,
            ),
            (Self::Visible, Value::Flag(v)) => {
                set_flag(&mut cfg.visible, v, "visible", Effect::Resize)
            }
            (Self::AutoSave, Value::Flag(v)) => {
                cfg.auto_save = v;
                CommandResult::ok(format!("auto-save={}", on_off(v)), Vec::new())
            }
            (Self::OwnUser, Value::Flag(v)) => {
                set_flag(&mut cfg.show_own_user, v, "own-user", Effect::Resize)
            }
            (Self::OffsetX, Value::Num(v)) => set_num(
                &mut cfg.offset_x,
                v as i32,
                OFFSETS,
                "offset-x",
                Effect::Reanchor,
            ),
            (Self::OffsetY, Value::Num(v)) => set_num(
                &mut cfg.offset_y,
                v as i32,
                OFFSETS,
                "offset-y",
                Effect::Reanchor,
            ),
            (Self::OffsetMin, Value::Num(v)) => {
                if OFFSETS.contains(v as i32) && (v as i32) < cfg.offset_max {
                    cfg.offset_min = v as i32;
                    CommandResult::ok(format!("offset-min={v}"), vec![Effect::Resize])
                } else {
                    CommandResult::err(format!(
                        "error: offset-min must stay below offset-max ({})",
                        cfg.offset_max
                    ))
                }
            }
            (Self::OffsetMax, Value::Num(v)) => {
                if OFFSETS.contains(v as i32) && (v as i32) > cfg.offset_min {
                    cfg.offset_max = v as i32;
                    CommandResult::ok(format!("offset-max={v}"), vec![Effect::Resize])
                } else {
                    CommandResult::err(format!(
                        "error: offset-max must stay above offset-min ({})",
                        cfg.offset_min
                    ))
                }
            }
            (Self::Width, Value::Num(v)) => {
                set_num(&mut cfg.width, v as u32, WIDTH, "width", Effect::Resize)
            }
            (Self::Scale, Value::Num(v)) => {
                set_num(&mut cfg.scale, v as u8, SCALE, "scale", Effect::Resize)
            }
            (Self::AvatarSize, Value::Num(v)) => set_num(
                &mut cfg.avatar_size,
                v as u32,
                AVATAR_SIZE,
                "avatar-size",
                Effect::Resize,
            ),
            (Self::TextSize, Value::Num(v)) => set_num(
                &mut cfg.text_size,
                v as u32,
                TEXT_SIZE,
                "text-size",
                Effect::Resize,
            ),
            (Self::Spacing, Value::Num(v)) => set_num(
                &mut cfg.spacing,
                v as u32,
                SPACING,
                "spacing",
                Effect::Resize,
            ),
            (Self::MaxName, Value::Num(v)) => set_num(
                &mut cfg.max_username_length,
                v as usize,
                MAX_NAME,
                "max-name",
                Effect::Resize,
            ),
            (Self::Opacity, Value::Num(v)) => set_pct(&mut cfg.opacity, v as u8, "opacity"),
            (Self::AvatarOpacity, Value::Num(v)) => {
                set_pct(&mut cfg.avatar_opacity, v as u8, "avatar-opacity")
            }
            (Self::TextOpacity, Value::Num(v)) => {
                set_pct(&mut cfg.text_opacity, v as u8, "text-opacity")
            }
            (Self::BoxOpacity, Value::Num(v)) => {
                set_pct(&mut cfg.box_opacity, v as u8, "box-opacity")
            }
            (Self::HoverOpacity, Value::Num(v)) => {
                set_pct(&mut cfg.hover_opacity, v as u8, "hover-opacity")
            }
            (Self::SpeakingColor, Value::Color(c)) => {
                set_color(&mut cfg.speaking_color, c, "speaking-color")
            }
            (Self::TextColor, Value::Color(c)) => set_color(&mut cfg.text_color, c, "text-color"),
            (Self::BoxColor, Value::Color(c)) => set_color(&mut cfg.box_color, c, "box-color"),
            (key, value) => CommandResult::err(format!(
                "error: invalid value {value:?} for {} {}",
                key.name(),
                hint_for(key)
            )),
        }
    }
}

pub fn corner_of(horizontal: HorizontalAnchor, vertical: VerticalAnchor) -> Corner {
    match (horizontal, vertical) {
        (HorizontalAnchor::Right, VerticalAnchor::Top) => Corner::TopRight,
        (HorizontalAnchor::Left, VerticalAnchor::Bottom) => Corner::BottomLeft,
        (HorizontalAnchor::Right, VerticalAnchor::Bottom) => Corner::BottomRight,
        _ => Corner::TopLeft,
    }
}

impl Corner {
    fn next(self) -> Self {
        let index = CORNER_CYCLE.iter().position(|c| *c == self).unwrap_or(0);
        CORNER_CYCLE[(index + 1) % CORNER_CYCLE.len()]
    }
}

/// One control-socket command line. Parses from exactly the text the CLI,
/// keybinds, and GUI send; `Display` emits the canonical form. Config
/// settings all live behind `get <key>` / `set <key> [value]`; a bare
/// `set <key>` cycles enum keys and flips flags. Daemon-side commands —
/// `save`, `dump`, `status`, `help`, `get`, `restart`, `quit`, and
/// `set monitor` — need runtime state, so the shell intercepts and answers
/// them itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    // Daemon-side: answered by the shell, they never touch the config.
    Save,
    Dump,
    Status,
    Help,
    /// Re-exec the daemon so startup-time decisions (layer surface,
    /// authentication backend) are made again.
    Restart,

    // Config-wide actions.
    /// Reset every key to its default (keeps the monitor choice).
    ResetAll,
    /// Reset one group's keys to their defaults (keeps the monitor).
    ResetGroup(Group),
    /// Re-read config.toml from disk.
    Reload,

    // Runtime placement actions.
    MoveEdge(Edge),
    Nudge(i32, i32),

    // The get/set surface.
    Get(Key),
    Set(Key, Value),

    /// Ask the daemon to shut down cleanly after answering. Daemon-side:
    /// the shell replies [`Command::QUIT_REPLY`] and stops the runtime;
    /// it never reaches config application.
    Quit,
}

impl Command {
    /// Wire reply to the `quit` command. Part of the byte-stable protocol
    /// between clients and the daemon; pinned by tests, never reworded.
    pub const QUIT_REPLY: &'static str = "quitting";
}

impl FromStr for Command {
    type Err = String;

    fn from_str(line: &str) -> Result<Self, Self::Err> {
        let mut parts = line.split_whitespace();
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next();
        let unknown = || format!("error: unknown command {cmd:?} (try 'help')");
        let unknown_key = || format!("error: unknown key {arg:?} (try 'help')");

        match cmd {
            "save" => Ok(Self::Save),
            "dump" => Ok(Self::Dump),
            "status" => Ok(Self::Status),
            "help" => Ok(Self::Help),
            "restart" => Ok(Self::Restart),
            "quit" => Ok(Self::Quit),
            "reload" => Ok(Self::Reload),
            "reset" => match arg {
                None => Ok(Self::ResetAll),
                Some(word) => Group::parse(word)
                    .map(Self::ResetGroup)
                    .ok_or_else(|| "error: reset <position|layout|opacity|colors>".to_string()),
            },
            "move" => match arg {
                Some("left") => Ok(Self::MoveEdge(Edge::Left)),
                Some("right") => Ok(Self::MoveEdge(Edge::Right)),
                Some("center") => Ok(Self::MoveEdge(Edge::Center)),
                Some("top") => Ok(Self::MoveEdge(Edge::Top)),
                Some("bottom") => Ok(Self::MoveEdge(Edge::Bottom)),
                _ => Err("error: move <left|right|center|top|bottom>".to_string()),
            },
            "nudge" => {
                let (Some(dx), Some(dy)) = (
                    arg.and_then(|v| v.parse().ok()),
                    parts.next().and_then(|v| v.parse().ok()),
                ) else {
                    return Err("error: nudge <dx> <dy>".to_string());
                };
                Ok(Self::Nudge(dx, dy))
            }
            "get" => {
                let Some(name) = arg else {
                    return Err("error: get <key> (try 'help')".to_string());
                };
                Key::parse(name).map(Self::Get).ok_or_else(unknown_key)
            }
            "set" => {
                let Some(name) = arg else {
                    return Err("error: set <key> [value] (try 'help')".to_string());
                };
                let key = Key::parse(name).ok_or_else(unknown_key)?;
                key.parse_value(parts.next())
                    .map(|value| Self::Set(key, value))
            }
            _ => Err(unknown()),
        }
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Save => f.write_str("save"),
            Self::Dump => f.write_str("dump"),
            Self::Status => f.write_str("status"),
            Self::Help => f.write_str("help"),
            Self::Restart => f.write_str("restart"),
            Self::Quit => f.write_str("quit"),
            Self::Reload => f.write_str("reload"),
            Self::ResetAll => f.write_str("reset"),
            Self::ResetGroup(group) => write!(f, "reset {group}"),
            Self::MoveEdge(edge) => {
                let word = match edge {
                    Edge::Left => "left",
                    Edge::Right => "right",
                    Edge::Center => "center",
                    Edge::Top => "top",
                    Edge::Bottom => "bottom",
                };
                write!(f, "move {word}")
            }
            Self::Nudge(dx, dy) => write!(f, "nudge {dx} {dy}"),
            Self::Get(key) => write!(f, "get {}", key.name()),
            Self::Set(key, Value::Cycle) => write!(f, "set {}", key.name()),
            Self::Set(key, value) => write!(f, "set {} {value}", key.name()),
        }
    }
}

impl Command {
    /// Apply a config mutation or answer a read. Daemon-side commands —
    /// `save`, `dump`, `status`, `help`, `get`, `restart`, and
    /// `set monitor` — need live state or IO, so the shell answers them
    /// itself and never routes them here (reaching this match is the
    /// fallback that reports "not a config command").
    pub fn apply_config(self, config: &mut Config) -> CommandResult {
        match self {
            Self::Get(key) => CommandResult::ok(key.get(config), vec![]),
            Self::Set(key, value) => key.apply(config, value),
            Self::ResetAll => {
                let monitor = config.monitor.clone();
                *config = Config::default();
                config.monitor = monitor;
                CommandResult::ok("reset", vec![Effect::Reanchor, Effect::Resize])
            }
            Self::ResetGroup(group) => {
                let defaults = Config::default();
                for key in Key::ALL {
                    if key == Key::Monitor || key.group() != group {
                        continue;
                    }
                    if key == Key::ShowOnFullscreen {
                        config.show_on_fullscreen = defaults.show_on_fullscreen;
                        continue;
                    }
                    key.apply(config, key.value_of(&defaults));
                }
                CommandResult::ok(
                    format!("reset {group}"),
                    vec![Effect::Reanchor, Effect::Resize],
                )
            }
            Self::Reload => {
                *config = config::load();
                CommandResult::ok("reloaded", vec![Effect::Reanchor, Effect::Resize])
            }
            Self::MoveEdge(edge) => {
                match edge {
                    Edge::Left => config.horizontal = HorizontalAnchor::Left,
                    Edge::Right => config.horizontal = HorizontalAnchor::Right,
                    Edge::Center => config.horizontal = HorizontalAnchor::Center,
                    Edge::Top => config.vertical = VerticalAnchor::Top,
                    Edge::Bottom => config.vertical = VerticalAnchor::Bottom,
                }
                CommandResult::ok(format!("moved {edge}"), vec![Effect::Reanchor])
            }
            Self::Nudge(dx, dy) => CommandResult::ok("nudged", vec![Effect::Nudge(dx, dy)]),
            Self::Save | Self::Dump | Self::Status | Self::Help | Self::Restart | Self::Quit => {
                CommandResult::err("error: not a config command")
            }
        }
    }
}

impl fmt::Display for Edge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Center => "center",
            Self::Top => "top",
            Self::Bottom => "bottom",
        })
    }
}

fn on_off(v: bool) -> &'static str {
    if v { "on" } else { "off" }
}

fn set_flag(field: &mut bool, v: bool, name: &str, effect: Effect) -> CommandResult {
    *field = v;
    CommandResult::ok(format!("{name}={}", on_off(v)), vec![effect])
}

fn set_pct(field: &mut u8, v: u8, name: &str) -> CommandResult {
    if !OPACITY.contains(v) {
        return CommandResult::err(format!("error: {name} {}", OPACITY.hint()));
    }
    *field = v;
    CommandResult::ok(format!("{name}={v}"), vec![Effect::Resize])
}

fn set_num<T>(field: &mut T, v: T, bounds: Bounds<T>, name: &str, effect: Effect) -> CommandResult
where
    T: Ord + fmt::Display + Copy,
{
    if !bounds.contains(v) {
        return CommandResult::err(format!("error: {name} {}", bounds.hint()));
    }
    *field = v;
    CommandResult::ok(format!("{name}={v}"), vec![effect])
}

fn set_color(field: &mut HexColor, value: HexColor, name: &str) -> CommandResult {
    *field = value;
    CommandResult::ok(format!("{name}={value}"), vec![Effect::Resize])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AnchorMode;

    fn apply(line: &str, cfg: &mut Config) -> CommandResult {
        line.parse::<Command>().expect("parses").apply_config(cfg)
    }

    /// Like [`apply`] but keeps parse errors as an `Err` reply, mirroring
    /// how the shell surfaces them.
    fn run(line: &str, cfg: &mut Config) -> Result<CommandResult, String> {
        line.parse::<Command>().map(|c| c.apply_config(cfg))
    }

    fn parse_err(line: &str) -> String {
        line.parse::<Command>().expect_err("errors")
    }

    #[test]
    fn get_reads_any_key_in_canonical_form() {
        let cfg = Config::default();
        assert_eq!(
            run("get opacity", &mut cfg.clone()).unwrap().reply,
            "opacity=100"
        );
        assert_eq!(
            run("get position", &mut cfg.clone()).unwrap().reply,
            "position=top-left"
        );
        assert_eq!(
            run("get monitor", &mut cfg.clone()).unwrap().reply,
            "monitor=active"
        );
        assert_eq!(
            run("get speaking-color", &mut cfg.clone()).unwrap().reply,
            "speaking-color=#22c55e"
        );
        assert_eq!(run("get rtl", &mut cfg.clone()).unwrap().reply, "rtl=off");
    }

    #[test]
    fn set_then_get_roundtrips_numeric_keys() {
        let mut cfg = Config::default();
        let r = apply("set opacity 42", &mut cfg);
        assert_eq!(r.reply, "opacity=42");
        assert_eq!(cfg.opacity, 42);
        assert_eq!(apply("get opacity", &mut cfg).reply, "opacity=42");
    }

    #[test]
    fn numeric_set_rejects_out_of_range_with_bounds_hint() {
        let cfg = Config::default();
        assert_eq!(parse_err("set width 1000"), "error: width <200-600>");
        assert_eq!(parse_err("set scale 10"), "error: scale <50-200>");
        assert_eq!(
            parse_err("set offset-x 99999"),
            format!("error: offset-x {}", OFFSETS.hint())
        );
        // Nothing was applied.
        assert_eq!(cfg.width, Config::default().width);
    }

    #[test]
    fn numeric_set_requires_a_value() {
        assert_eq!(parse_err("set width"), "error: width <200-600>");
        assert_eq!(parse_err("set box-color"), "error: box-color <#rrggbb>");
    }

    #[test]
    fn flag_values_parse_and_bare_form_flips() {
        let mut cfg = Config::default();
        assert_eq!(
            apply("set talking-only on", &mut cfg).reply,
            "talking-only=on"
        );
        assert!(cfg.show_only_talking_users);
        assert_eq!(apply("set own-user off", &mut cfg).reply, "own-user=off");
        assert!(!cfg.show_own_user);

        // Bare form flips: default off -> on -> off.
        assert_eq!(
            apply("set talking-only", &mut cfg).reply,
            "talking-only=off"
        );
        assert_eq!(apply("set talking-only", &mut cfg).reply, "talking-only=on");
    }

    #[test]
    fn mutating_command_persists_when_auto_save_is_on() {
        let cases = [
            Command::Set(Key::Opacity, Value::Num(42)),
            Command::MoveEdge(Edge::Left),
            Command::ResetAll,
            Command::ResetGroup(Group::Opacity),
        ];
        for cmd in cases {
            assert!(
                should_persist(&cmd, true),
                "{cmd:?} must persist under autosave"
            );
        }
    }

    #[test]
    fn mutating_command_leaves_disk_untouched_when_auto_save_is_off() {
        let cases = [
            Command::Set(Key::Opacity, Value::Num(42)),
            Command::MoveEdge(Edge::Left),
            Command::ResetAll,
            Command::ResetGroup(Group::Opacity),
        ];
        for cmd in cases {
            assert!(
                !should_persist(&cmd, false),
                "{cmd:?} must not persist with autosave off"
            );
        }
    }

    #[test]
    fn nudge_under_auto_save_never_triggers_a_disk_write() {
        // Nudge only shifts the surface at runtime (Effect::Nudge); the
        // persisted config is untouched, so autosave must not write disk.
        assert!(!should_persist(&Command::Nudge(3, -2), true));
    }

    #[test]
    fn non_mutating_commands_never_persist() {
        let read_only = [
            Command::Status,
            Command::Help,
            Command::Dump,
            Command::Get(Key::Width),
            Command::Reload,
            Command::Restart,
            Command::Nudge(3, -2),
            Command::Quit,
        ];
        for cmd in read_only {
            assert!(!should_persist(&cmd, true), "{cmd:?} never persists");
            assert!(!should_persist(&cmd, false), "{cmd:?} never persists");
        }
        // The explicit force-write is the one command that persists even
        // while the switch is off — that is its entire purpose.
        assert!(should_persist(&Command::Save, false));
    }

    #[test]
    fn auto_save_key_roundtrips_through_the_wire_grammar() {
        let mut cfg = Config::default();
        // Explicit values follow the shared bool convention...
        assert_eq!(apply("set auto-save off", &mut cfg).reply, "auto-save=off");
        assert!(!cfg.auto_save);
        assert_eq!(apply("set auto-save on", &mut cfg).reply, "auto-save=on");
        assert!(cfg.auto_save);
        // ...and the bare form flips like every other cycle-able key.
        assert_eq!(apply("set auto-save", &mut cfg).reply, "auto-save=off");
        assert!(!cfg.auto_save);
        // Canonical wire text round-trips through parse unchanged.
        assert_eq!(
            "set auto-save on".parse::<Command>().unwrap(),
            Command::Set(Key::AutoSave, Value::Flag(true))
        );
    }

    #[test]
    fn visible_key_roundtrips_through_the_wire_grammar() {
        let mut cfg = Config::default();
        // Explicit values follow the shared bool convention...
        assert_eq!(apply("set visible off", &mut cfg).reply, "visible=off");
        assert!(!cfg.visible);
        assert_eq!(apply("set visible on", &mut cfg).reply, "visible=on");
        assert!(cfg.visible);
        // ...and the bare form flips like every other cycle-able key.
        assert_eq!(apply("set visible", &mut cfg).reply, "visible=off");
        assert!(!cfg.visible);
        // Canonical wire text round-trips through parse unchanged.
        assert_eq!(
            "set visible off".parse::<Command>().unwrap(),
            Command::Set(Key::Visible, Value::Flag(false))
        );
    }

    #[test]
    fn flag_rejects_garbage_argument() {
        let err = parse_err("set rtl maybe");
        assert_eq!(err, "error: rtl <on|off>");
    }

    #[test]
    fn corner_cycles_through_all_four_and_right_side_enables_rtl() {
        let mut cfg = Config::default();
        let sequence = [
            ("top-right", true),
            ("bottom-left", false),
            ("bottom-right", true),
            ("top-left", false),
        ];
        for (word, rtl) in sequence {
            let r = apply("set position", &mut cfg);
            assert!(
                r.reply.starts_with(&format!("position={word}")),
                "{}",
                r.reply
            );
            assert_eq!(cfg.rtl, rtl);
        }
    }

    #[test]
    fn corner_accepts_full_words_and_short_aliases() {
        let mut cfg = Config::default();
        apply("set position br", &mut cfg);
        assert_eq!(
            (cfg.horizontal, cfg.vertical),
            (HorizontalAnchor::Right, VerticalAnchor::Bottom)
        );
        apply("set position top-right", &mut cfg);
        assert_eq!(cfg.horizontal, HorizontalAnchor::Right);
    }

    #[test]
    fn anchor_cycles_auto_top_bottom_and_overrides_glue_edge() {
        let mut cfg = Config::default();
        assert_eq!(apply("set anchor", &mut cfg).reply, "anchor=top");
        assert_eq!(cfg.anchor, AnchorMode::Top);
        apply("set anchor bottom", &mut cfg);
        assert_eq!(cfg.anchor, AnchorMode::Bottom);
        apply("set anchor", &mut cfg);
        assert_eq!(cfg.anchor, AnchorMode::Auto);
    }

    #[test]
    fn monitor_key_parses_active_and_named_targets() {
        // Parse side: named, active, and bare-cycle forms all parse.
        let cmd: Command = "set monitor DP-2".parse().unwrap();
        assert_eq!(
            cmd,
            Command::Set(
                Key::Monitor,
                Value::Target(MonitorTarget::Named("DP-2".to_string()))
            )
        );
        let cmd: Command = "set monitor active".parse().unwrap();
        assert_eq!(
            cmd,
            Command::Set(Key::Monitor, Value::Target(MonitorTarget::Active))
        );
        let cmd: Command = "set monitor".parse().unwrap();
        assert_eq!(cmd, Command::Set(Key::Monitor, Value::Cycle));
        // Apply side: the shell intercepts monitor changes before this
        // point, so direct application deterministically reports the
        // fallback error instead of touching config.
        let mut cfg = Config::default();
        assert_eq!(
            apply("set monitor DP-2", &mut cfg).reply,
            "error: not a config command"
        );
    }

    #[test]
    fn colors_parse_in_both_hex_lengths_and_display_canonical() {
        let mut cfg = Config::default();
        apply("set text-color #0ff", &mut cfg);
        assert_eq!(
            apply("get text-color", &mut cfg).reply,
            "text-color=#00ffff"
        );
        assert_eq!(
            parse_err("set text-color nope"),
            "error: text-color <#rrggbb>"
        );
    }

    #[test]
    fn unknown_keys_and_commands_name_the_problem() {
        assert!(parse_err("get nonsense").starts_with("error: unknown key"));
        assert!(parse_err("set nonsense 1").starts_with("error: unknown key"));
        assert!(parse_err("explode").starts_with("error: unknown command"));
        assert!(parse_err("set").starts_with("error: set"));
        assert!(parse_err("get").starts_with("error: get"));
    }

    #[test]
    fn reset_group_restores_only_that_group() {
        let mut cfg = Config::default();
        apply("set width 500", &mut cfg);
        apply("set opacity 30", &mut cfg);
        apply("reset layout", &mut cfg);
        assert_eq!(cfg.width, Config::default().width);
        assert_eq!(cfg.opacity, 30); // untouched group
    }

    #[test]
    fn reset_group_never_touches_monitor() {
        let mut cfg = Config {
            monitor: Some("DP-2".into()),
            ..Config::default()
        };
        apply("reset position", &mut cfg);
        assert_eq!(cfg.monitor.as_deref(), Some("DP-2"));
        assert_eq!(cfg.horizontal, Config::default().horizontal);
    }

    #[test]
    fn reset_all_keeps_monitor_and_restores_everything_else() {
        let mut cfg = Config::default();
        apply("set opacity 20", &mut cfg);
        apply("set width 600", &mut cfg);
        cfg.monitor = Some("DP-2".into());
        let r = apply("reset", &mut cfg);
        assert_eq!(r.reply, "reset");
        assert_eq!(cfg.opacity, Config::default().opacity);
        assert_eq!(cfg.width, Config::default().width);
        assert_eq!(cfg.monitor.as_deref(), Some("DP-2"));
    }

    #[test]
    fn display_is_canonical_wire_text() {
        let cases = [
            (Command::Save, "save"),
            (Command::Dump, "dump"),
            (Command::Status, "status"),
            (Command::Help, "help"),
            (Command::Restart, "restart"),
            (Command::Reload, "reload"),
            (Command::ResetAll, "reset"),
            (Command::ResetGroup(Group::Opacity), "reset opacity"),
            (Command::MoveEdge(Edge::Left), "move left"),
            (Command::Nudge(3, -4), "nudge 3 -4"),
            (Command::Get(Key::Width), "get width"),
            (Command::Set(Key::Opacity, Value::Num(42)), "set opacity 42"),
            (Command::Set(Key::Rtl, Value::Cycle), "set rtl"),
            (
                Command::Set(Key::Position, Value::Corner(Corner::BottomRight)),
                "set position bottom-right",
            ),
        ];
        for (command, wire) in cases {
            assert_eq!(command.to_string(), wire);
            assert_eq!(wire.parse::<Command>().unwrap(), command, "{wire}");
        }
    }

    #[test]
    fn move_edge_changes_one_axis_only() {
        let mut cfg = Config::default();
        let r = apply("move center", &mut cfg);
        assert_eq!(cfg.horizontal, HorizontalAnchor::Center);
        assert_eq!(cfg.vertical, VerticalAnchor::Top);
        assert_eq!(r.reply, "moved center");
    }

    #[test]
    fn middle_is_not_a_move_token_and_the_error_keeps_the_full_list() {
        // `middle` was an undocumented alias; the wire grammar accepts only
        // the five canonical tokens now ("center" stays). The parse error is
        // a socket reply, so it keeps the FULL token list (spec D2).
        let err = parse_err("move middle");
        assert_eq!(err, "error: move <left|right|center|top|bottom>");
        for token in ["left", "right", "center", "top", "bottom"] {
            let word = format!("move {token}");
            assert!(word.parse::<Command>().is_ok(), "{token} must still parse");
        }
    }

    #[test]
    fn nudge_reports_without_touching_config() {
        let mut cfg = Config::default();
        let before = cfg.clone();
        let r = apply("nudge 12 -8", &mut cfg);
        assert_eq!(r.reply, "nudged");
        assert_eq!(r.effects, vec![Effect::Nudge(12, -8)]);
        assert_eq!(cfg, before);
    }

    #[test]
    fn quit_wire_command_is_pinned_byte_for_byte() {
        // The whole quit contract in one place: the word parses, Display
        // emits it back canonically, and the reply text is part of the
        // byte-stable wire protocol.
        assert_eq!(
            "quit".parse::<Command>().unwrap(),
            Command::Quit,
            "the wire word is exactly 'quit'"
        );
        assert_eq!(Command::Quit.to_string(), "quit");
        assert_eq!(Command::QUIT_REPLY, "quitting");
    }

    #[test]
    fn daemon_side_commands_are_not_config_commands() {
        let cfg = Config::default();
        for line in ["save", "dump", "status", "help", "restart", "quit"] {
            // These parse fine but are answered by the shell, never by
            // apply_config.
            let result = run(line, &mut cfg.clone()).unwrap_or_else(|e| {
                panic!("{line} should parse: {e}");
            });
            assert_eq!(result.reply, "error: not a config command");
            assert!(result.effects.is_empty());
        }
    }

    #[test]
    fn key_table_covers_every_key_with_unique_names() {
        let mut names: Vec<&str> = Key::ALL.iter().map(|k| k.name()).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "duplicate key names");
        for key in Key::ALL {
            assert_eq!(Key::parse(key.name()), Some(key));
        }
    }

    #[test]
    fn every_key_belongs_to_a_real_group() {
        for key in Key::ALL {
            assert!(Group::ALL.contains(&key.group()), "{:?}", key);
        }
    }

    #[test]
    fn num_bounds_match_the_shared_bounds_table() {
        use crate::config::AVATAR_SIZE;
        use crate::config::MAX_NAME;
        use crate::config::SCALE;
        use crate::config::SPACING;
        use crate::config::TEXT_SIZE;
        use crate::config::WIDTH;
        assert_eq!(
            Key::OffsetX.num_bounds(),
            Some((OFFSETS.min as i64, OFFSETS.max as i64))
        );
        assert_eq!(
            Key::Opacity.num_bounds(),
            Some((OPACITY.min as i64, OPACITY.max as i64))
        );
        assert_eq!(
            Key::Width.num_bounds(),
            Some((WIDTH.min as i64, WIDTH.max as i64))
        );
        assert_eq!(
            Key::Scale.num_bounds(),
            Some((SCALE.min as i64, SCALE.max as i64))
        );
        assert_eq!(
            Key::AvatarSize.num_bounds(),
            Some((AVATAR_SIZE.min as i64, AVATAR_SIZE.max as i64))
        );
        assert_eq!(
            Key::TextSize.num_bounds(),
            Some((TEXT_SIZE.min as i64, TEXT_SIZE.max as i64))
        );
        assert_eq!(
            Key::Spacing.num_bounds(),
            Some((SPACING.min as i64, SPACING.max as i64))
        );
        assert_eq!(
            Key::MaxName.num_bounds(),
            Some((MAX_NAME.min as i64, MAX_NAME.max as i64))
        );
        assert_eq!(Key::Rtl.num_bounds(), None);
    }

    #[test]
    fn slider_windows_offset_x_y_follow_configured_range() {
        let cfg = Config {
            offset_min: -100,
            offset_max: 100,
            ..Config::default()
        };
        assert_eq!(Key::OffsetX.slider_bounds(&cfg), Some((-100.0, 100.0)));
        assert_eq!(Key::OffsetMin.slider_bounds(&cfg), None);
        assert_eq!(Key::Opacity.slider_bounds(&cfg), Some((0.0, 100.0)));
    }

    #[test]
    fn new_keys_have_wire_names_groups_and_bounds() {
        assert_eq!(Key::ShowOnFullscreen.name(), "show-on-fullscreen");
        assert_eq!(Key::DimOnHover.name(), "dim-on-hover");
        assert_eq!(Key::HoverOpacity.name(), "hover-opacity");
        assert_eq!(Key::ShowOnFullscreen.group(), Group::Layout);
        assert_eq!(Key::DimOnHover.group(), Group::Layout);
        assert_eq!(Key::HoverOpacity.group(), Group::Opacity);
        assert_eq!(
            Key::HoverOpacity.num_bounds(),
            Some((OPACITY.min as i64, OPACITY.max as i64))
        );
        assert_eq!(Key::ShowOnFullscreen.num_bounds(), None);
        assert_eq!(Key::DimOnHover.num_bounds(), None);
        assert_eq!(
            Key::HoverOpacity.slider_bounds(&Config::default()),
            Some((0.0, 100.0))
        );
    }

    #[test]
    fn new_keys_parse_values_and_cycle_correctly() {
        assert_eq!(
            "set show-on-fullscreen on".parse::<Command>().unwrap(),
            Command::Set(Key::ShowOnFullscreen, Value::Flag(true))
        );
        assert_eq!(
            "set show-on-fullscreen off".parse::<Command>().unwrap(),
            Command::Set(Key::ShowOnFullscreen, Value::Flag(false))
        );
        assert_eq!(
            "set show-on-fullscreen".parse::<Command>().unwrap(),
            Command::Set(Key::ShowOnFullscreen, Value::Cycle)
        );
        assert_eq!(
            "set dim-on-hover on".parse::<Command>().unwrap(),
            Command::Set(Key::DimOnHover, Value::Flag(true))
        );
        assert_eq!(
            "set dim-on-hover".parse::<Command>().unwrap(),
            Command::Set(Key::DimOnHover, Value::Cycle)
        );
        assert_eq!(
            parse_err("set dim-on-hover maybe"),
            "error: dim-on-hover <on|off>"
        );
        assert_eq!(
            "set hover-opacity 40".parse::<Command>().unwrap(),
            Command::Set(Key::HoverOpacity, Value::Num(40))
        );
        assert_eq!(
            parse_err("set hover-opacity"),
            "error: hover-opacity <0-100>"
        );
        assert_eq!(
            parse_err("set hover-opacity 200"),
            "error: hover-opacity <0-100>"
        );
    }

    #[test]
    fn new_keys_apply_and_reset_groups() {
        let mut cfg = Config::default();
        assert_eq!(
            apply("set dim-on-hover on", &mut cfg).reply,
            "dim-on-hover=on"
        );
        assert!(cfg.dim_on_hover);
        assert_eq!(
            apply("set dim-on-hover", &mut cfg).reply,
            "dim-on-hover=off"
        );
        assert_eq!(
            apply("set hover-opacity 55", &mut cfg).reply,
            "hover-opacity=55"
        );
        assert_eq!(cfg.hover_opacity, 55);
        assert_eq!(
            apply("set show-on-fullscreen on", &mut cfg).reply,
            "error: not a config command"
        );
        let mut cfg2 = Config {
            dim_on_hover: true,
            hover_opacity: 20,
            show_on_fullscreen: false,
            visible: false,
            width: 500,
            opacity: 30,
            ..Config::default()
        };
        apply("reset layout", &mut cfg2);
        assert_eq!(cfg2.dim_on_hover, Config::default().dim_on_hover);
        assert_eq!(
            cfg2.show_on_fullscreen,
            Config::default().show_on_fullscreen
        );
        assert_eq!(cfg2.visible, Config::default().visible);
        assert_eq!(cfg2.width, Config::default().width);
        assert_eq!(cfg2.hover_opacity, 20);
        assert_eq!(cfg2.opacity, 30);
        apply("reset opacity", &mut cfg2);
        assert_eq!(cfg2.hover_opacity, Config::default().hover_opacity);
        assert_eq!(cfg2.opacity, Config::default().opacity);
    }

    #[test]
    fn new_keys_display_canonical_and_get_roundtrip() {
        assert_eq!(
            Command::Set(Key::ShowOnFullscreen, Value::Flag(true)).to_string(),
            "set show-on-fullscreen on"
        );
        assert_eq!(
            Command::Set(Key::DimOnHover, Value::Cycle).to_string(),
            "set dim-on-hover"
        );
        assert_eq!(
            Command::Set(Key::HoverOpacity, Value::Num(40)).to_string(),
            "set hover-opacity 40"
        );
        let cfg = Config::default();
        assert_eq!(
            Key::ShowOnFullscreen.get(&cfg),
            format!(
                "show-on-fullscreen={}",
                if cfg.show_on_fullscreen { "on" } else { "off" }
            )
        );
        assert_eq!(Key::HoverOpacity.get(&cfg), "hover-opacity=40");
    }
}
