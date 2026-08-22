//! Config: `$XDG_CONFIG_HOME/hyprlay/config.toml`.
//! All numeric fields are clamped on load so a hand-edited file can never
//! produce a broken overlay.

use std::fmt;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use crate::domain::HexColor;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum HorizontalAnchor {
    #[default]
    Left,
    Right,
    Center,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum VerticalAnchor {
    #[default]
    Top,
    Bottom,
}

/// Which screen edge the surface glues to vertically. `Auto` derives it
/// from `vertical` (top corners glue top, bottom corners glue bottom);
/// explicit values override — this decides the direction the user list
/// grows when new participants join: anchored top grows down, anchored
/// bottom grows up, so the overlay never runs off screen.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[serde(rename_all = "lowercase")]
pub enum AnchorMode {
    #[default]
    Auto,
    Top,
    Bottom,
}

/// Hard safety bound for offsets and the offset slider range.
pub const OFFSET_LIMIT: i32 = 4000;

/// Inclusive range for one numeric value. This is the single source of
/// truth shared by parsers, appliers, config-file clamps, slider ranges,
/// and help text — change a bound here and every surface follows.
#[derive(Debug, Clone, Copy)]
pub struct Bounds<T> {
    pub min: T,
    pub max: T,
}

impl<T: Ord + fmt::Display + Copy> Bounds<T> {
    pub fn contains(&self, v: T) -> bool {
        v >= self.min && v <= self.max
    }

    pub fn clamp_value(&self, v: T) -> T {
        v.clamp(self.min, self.max)
    }

    /// The `<min-max>` fragment used in CLI error replies and help text.
    pub fn hint(&self) -> String {
        format!("<{}-{}>", self.min, self.max)
    }
}

pub const OPACITY: Bounds<u8> = Bounds { min: 0, max: 100 };
pub const WIDTH: Bounds<u32> = Bounds { min: 200, max: 600 };
pub const SCALE: Bounds<u8> = Bounds { min: 50, max: 200 };
pub const AVATAR_SIZE: Bounds<u32> = Bounds { min: 16, max: 64 };
pub const TEXT_SIZE: Bounds<u32> = Bounds { min: 8, max: 32 };
pub const SPACING: Bounds<u32> = Bounds { min: 0, max: 24 };
pub const MAX_NAME: Bounds<usize> = Bounds { min: 4, max: 64 };
pub const OFFSETS: Bounds<i32> = Bounds {
    min: -OFFSET_LIMIT,
    max: OFFSET_LIMIT,
};

#[derive(Clone, PartialEq, Debug)]
pub struct Config {
    pub horizontal: HorizontalAnchor,
    pub vertical: VerticalAnchor,
    /// Vertical glue-edge override; `Auto` follows `vertical`.
    pub anchor: AnchorMode,
    /// Distance in px from the anchored screen edges.
    pub offset_x: i32,
    pub offset_y: i32,
    /// Lower/upper bound of the offset inputs in the GUI, in px.
    pub offset_min: i32,
    pub offset_max: i32,
    /// Right-to-left layout: avatar on the right, name to its left.
    pub rtl: bool,
    /// Target output name; `None` = the active monitor at startup.
    /// (Layer-shell surfaces are bound to an output at creation, so the
    /// daemon restarts itself when this changes.)
    pub monitor: Option<String>,
    /// Panel width in logical px (200..=600).
    pub width: u32,
    /// Global scale in percent (50..=200).
    pub scale: u8,
    /// Overall opacity in percent (0..=100), multiplied into every part.
    pub opacity: u8,
    /// Per-part opacity in percent (0..=100), applied on top of `opacity`:
    /// profile picture, username text, and the username background chip.
    pub avatar_opacity: u8,
    pub text_opacity: u8,
    pub box_opacity: u8,
    pub max_username_length: usize,
    pub show_own_user: bool,
    pub show_only_talking_users: bool,
    /// Master visibility switch: false collapses the overlay to an empty
    /// surface while the daemon keeps running and tracking state.
    pub visible: bool,
    /// Persist every applied change to config.toml immediately. Off keeps
    /// changes session-only until an explicit `save`.
    pub auto_save: bool,
    /// Avatar diameter in logical px (16..=64).
    pub avatar_size: u32,
    /// Username font size in logical px (8..=32).
    pub text_size: u32,
    /// Gap between rows in logical px (0..=24).
    pub spacing: u32,
    /// Speaking ring color.
    pub speaking_color: HexColor,
    /// Username color.
    pub text_color: HexColor,
    /// Username chip background color.
    pub box_color: HexColor,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            horizontal: HorizontalAnchor::Left,
            vertical: VerticalAnchor::Top,
            anchor: AnchorMode::Auto,
            offset_x: 12,
            offset_y: 12,
            offset_min: -2000,
            offset_max: 2000,
            rtl: false,
            monitor: None,
            width: 300,
            scale: 100,
            opacity: 100,
            avatar_opacity: 100,
            text_opacity: 100,
            box_opacity: 90,
            max_username_length: 16,
            show_own_user: true,
            show_only_talking_users: false,
            visible: true,
            auto_save: true,
            avatar_size: 34,
            text_size: 14,
            spacing: 4,
            speaking_color: HexColor::from_rgb8(0x22, 0xc5, 0x5e),
            text_color: HexColor::from_rgb8(0xff, 0xff, 0xff),
            box_color: HexColor::from_rgb8(0x0d, 0x0d, 0x0f),
        }
    }
}

/// Effective per-part alphas: overall percent times each part's percent.
/// Derived data only — the config stays the single source of truth.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Alphas {
    pub overall: f32,
    pub avatar: f32,
    pub text: f32,
    pub box_bg: f32,
}

impl AnchorMode {
    /// auto -> top -> bottom -> auto, for bare `set anchor`.
    pub fn next(self) -> Self {
        match self {
            Self::Auto => Self::Top,
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Auto,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }
}

impl fmt::Display for AnchorMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hyprlay")
}

fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn load() -> Config {
    let parsed = fs::read_to_string(config_path()).and_then(|s| {
        toml::from_str(&s).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    });
    let cfg = match parsed {
        Ok(cfg) => cfg,
        // First run has no config file yet — that is normal.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(e) => {
            tracing::warn!(
                event = "config_load_failed",
                error = %e,
                "could not read or parse config; using defaults"
            );
            Config::default()
        }
    };
    let mut cfg = cfg;
    cfg.clamp();
    cfg
}

impl Config {
    /// The on-disk layout mirrors the GUI's four sections, so the file and
    /// the settings window teach the same organization:
    ///
    /// ```toml
    /// [position]
    /// horizontal = "left"
    /// vertical = "top"
    /// anchor = "auto"
    /// offset-x = 12
    /// ...
    /// ```
    fn to_file(&self) -> FileFormat {
        FileFormat {
            position: PositionTable {
                horizontal: Some(self.horizontal),
                vertical: Some(self.vertical),
                anchor: Some(self.anchor),
                offset_x: Some(self.offset_x),
                offset_y: Some(self.offset_y),
                offset_min: Some(self.offset_min),
                offset_max: Some(self.offset_max),
                rtl: Some(self.rtl),
                monitor: self.monitor.clone(),
            },
            layout: LayoutTable {
                width: Some(self.width),
                scale: Some(self.scale),
                avatar_size: Some(self.avatar_size),
                text_size: Some(self.text_size),
                spacing: Some(self.spacing),
                max_name: Some(self.max_username_length),
                talking_only: Some(self.show_only_talking_users),
                own_user: Some(self.show_own_user),
                visible: Some(self.visible),
                auto_save: Some(self.auto_save),
            },
            opacity: OpacityTable {
                overall: Some(self.opacity),
                avatar: Some(self.avatar_opacity),
                text: Some(self.text_opacity),
                box_: Some(self.box_opacity),
            },
            colors: ColorsTable {
                speaking: Some(self.speaking_color),
                text: Some(self.text_color),
                box_: Some(self.box_color),
            },
        }
    }

    fn from_file(file: FileFormat) -> Self {
        let d = Self::default();
        let p = file.position;
        let l = file.layout;
        let o = file.opacity;
        let c = file.colors;
        Self {
            horizontal: p.horizontal.unwrap_or(d.horizontal),
            vertical: p.vertical.unwrap_or(d.vertical),
            anchor: p.anchor.unwrap_or(d.anchor),
            offset_x: p.offset_x.unwrap_or(d.offset_x),
            offset_y: p.offset_y.unwrap_or(d.offset_y),
            offset_min: p.offset_min.unwrap_or(d.offset_min),
            offset_max: p.offset_max.unwrap_or(d.offset_max),
            rtl: p.rtl.unwrap_or(d.rtl),
            monitor: p.monitor.or(d.monitor),
            width: l.width.unwrap_or(d.width),
            scale: l.scale.unwrap_or(d.scale),
            avatar_size: l.avatar_size.unwrap_or(d.avatar_size),
            text_size: l.text_size.unwrap_or(d.text_size),
            spacing: l.spacing.unwrap_or(d.spacing),
            max_username_length: l.max_name.unwrap_or(d.max_username_length),
            show_only_talking_users: l.talking_only.unwrap_or(d.show_only_talking_users),
            show_own_user: l.own_user.unwrap_or(d.show_own_user),
            visible: l.visible.unwrap_or(d.visible),
            auto_save: l.auto_save.unwrap_or(d.auto_save),
            opacity: o.overall.unwrap_or(d.opacity),
            avatar_opacity: o.avatar.unwrap_or(d.avatar_opacity),
            text_opacity: o.text.unwrap_or(d.text_opacity),
            box_opacity: o.box_.unwrap_or(d.box_opacity),
            speaking_color: c.speaking.unwrap_or(d.speaking_color),
            text_color: c.text.unwrap_or(d.text_color),
            box_color: c.box_.unwrap_or(d.box_color),
        }
    }
}

// -- on-disk TOML shape (four tables mirroring the GUI sections) ------------

#[derive(Serialize, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
struct PositionTable {
    horizontal: Option<HorizontalAnchor>,
    vertical: Option<VerticalAnchor>,
    anchor: Option<AnchorMode>,
    offset_x: Option<i32>,
    offset_y: Option<i32>,
    offset_min: Option<i32>,
    offset_max: Option<i32>,
    rtl: Option<bool>,
    monitor: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
struct LayoutTable {
    width: Option<u32>,
    scale: Option<u8>,
    avatar_size: Option<u32>,
    text_size: Option<u32>,
    spacing: Option<u32>,
    max_name: Option<usize>,
    talking_only: Option<bool>,
    own_user: Option<bool>,
    visible: Option<bool>,
    auto_save: Option<bool>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct OpacityTable {
    overall: Option<u8>,
    avatar: Option<u8>,
    text: Option<u8>,
    #[serde(rename = "box")]
    box_: Option<u8>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct ColorsTable {
    speaking: Option<HexColor>,
    text: Option<HexColor>,
    #[serde(rename = "box")]
    box_: Option<HexColor>,
}

#[derive(Serialize, Deserialize, Default)]
struct FileFormat {
    #[serde(default)]
    position: PositionTable,
    #[serde(default)]
    layout: LayoutTable,
    #[serde(default)]
    opacity: OpacityTable,
    #[serde(default)]
    colors: ColorsTable,
}

impl serde::Serialize for Config {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_file().serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for Config {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        FileFormat::deserialize(deserializer).map(Self::from_file)
    }
}

impl Config {
    pub fn clamp(&mut self) {
        self.opacity = OPACITY.clamp_value(self.opacity);
        self.avatar_opacity = OPACITY.clamp_value(self.avatar_opacity);
        self.text_opacity = OPACITY.clamp_value(self.text_opacity);
        self.box_opacity = OPACITY.clamp_value(self.box_opacity);
        self.scale = SCALE.clamp_value(self.scale);
        self.width = WIDTH.clamp_value(self.width);
        self.max_username_length = MAX_NAME.clamp_value(self.max_username_length);
        self.offset_x = OFFSETS.clamp_value(self.offset_x);
        self.offset_y = OFFSETS.clamp_value(self.offset_y);
        self.offset_min = OFFSETS.clamp_value(self.offset_min);
        self.offset_max = OFFSETS.clamp_value(self.offset_max);
        if self.offset_min >= self.offset_max {
            let defaults = Config::default();
            self.offset_min = defaults.offset_min;
            self.offset_max = defaults.offset_max;
        }
        self.avatar_size = AVATAR_SIZE.clamp_value(self.avatar_size);
        self.text_size = TEXT_SIZE.clamp_value(self.text_size);
        self.spacing = SPACING.clamp_value(self.spacing);
    }

    pub fn save(&self) {
        let dir = config_dir();
        let write = || -> std::io::Result<()> {
            fs::create_dir_all(&dir)?;
            let payload = toml::to_string_pretty(self)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            fs::write(config_path(), payload)
        };
        if let Err(e) = write() {
            tracing::warn!(event = "config_save_failed", error = %e, "could not persist config");
        }
    }

    pub fn scale_f32(&self) -> f32 {
        self.scale as f32 / 100.0
    }

    pub fn alphas(&self) -> Alphas {
        let part = |pct: u8| self.opacity as f32 / 100.0 * pct as f32 / 100.0;
        Alphas {
            overall: self.opacity as f32 / 100.0,
            avatar: part(self.avatar_opacity),
            text: part(self.text_opacity),
            box_bg: part(self.box_opacity),
        }
    }
}

/// A named color template: all three overlay colors at once.
pub struct Palette {
    pub name: &'static str,
    pub speaking: HexColor,
    pub text: HexColor,
    pub box_bg: HexColor,
}

pub const PALETTES: &[Palette] = &[
    Palette {
        name: "Discord",
        speaking: HexColor::from_rgb8(0x22, 0xc5, 0x5e),
        text: HexColor::from_rgb8(0xff, 0xff, 0xff),
        box_bg: HexColor::from_rgb8(0x0d, 0x0d, 0x0f),
    },
    Palette {
        name: "Emerald",
        speaking: HexColor::from_rgb8(0x10, 0xb9, 0x81),
        text: HexColor::from_rgb8(0xd1, 0xfa, 0xe5),
        box_bg: HexColor::from_rgb8(0x06, 0x4e, 0x3b),
    },
    Palette {
        name: "Ocean",
        speaking: HexColor::from_rgb8(0x38, 0xbd, 0xf8),
        text: HexColor::from_rgb8(0xe0, 0xf2, 0xfe),
        box_bg: HexColor::from_rgb8(0x0c, 0x4a, 0x6e),
    },
    Palette {
        name: "Sunset",
        speaking: HexColor::from_rgb8(0xfb, 0x71, 0x85),
        text: HexColor::from_rgb8(0xff, 0xf1, 0xf2),
        box_bg: HexColor::from_rgb8(0x4c, 0x05, 0x19),
    },
    Palette {
        name: "Amber",
        speaking: HexColor::from_rgb8(0xf5, 0x9e, 0x0b),
        text: HexColor::from_rgb8(0xff, 0xfb, 0xeb),
        box_bg: HexColor::from_rgb8(0x45, 0x1a, 0x03),
    },
    Palette {
        name: "Mono",
        speaking: HexColor::from_rgb8(0xe5, 0xe7, 0xeb),
        text: HexColor::from_rgb8(0xf9, 0xfa, 0xfb),
        box_bg: HexColor::from_rgb8(0x1f, 0x29, 0x37),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_caps_opacity_at_100() {
        let mut cfg = Config {
            opacity: 150,
            ..Config::default()
        };
        cfg.clamp();
        assert_eq!(cfg.opacity, 100);
    }

    #[test]
    fn clamp_caps_per_part_opacity_at_100() {
        let mut cfg = Config {
            avatar_opacity: 200,
            text_opacity: 101,
            box_opacity: 255,
            ..Config::default()
        };
        cfg.clamp();
        assert_eq!(
            (cfg.avatar_opacity, cfg.text_opacity, cfg.box_opacity),
            (100, 100, 100)
        );
    }

    #[test]
    fn clamp_keeps_opacity_zero() {
        let mut cfg = Config {
            opacity: 0,
            ..Config::default()
        };
        cfg.clamp();
        assert_eq!(cfg.opacity, 0);
    }

    #[test]
    fn clamp_raises_scale_to_50_floor() {
        let mut cfg = Config {
            scale: 10,
            ..Config::default()
        };
        cfg.clamp();
        assert_eq!(cfg.scale, 50);
    }

    #[test]
    fn clamp_lowers_scale_to_200_ceiling() {
        let mut cfg = Config {
            scale: 250,
            ..Config::default()
        };
        cfg.clamp();
        assert_eq!(cfg.scale, 200);
    }

    #[test]
    fn clamp_bounds_avatar_and_text_sizes() {
        let mut cfg = Config {
            avatar_size: 512,
            text_size: 2,
            spacing: 90,
            ..Config::default()
        };
        cfg.clamp();
        assert_eq!(cfg.avatar_size, 64);
        assert_eq!(cfg.text_size, 8);
        assert_eq!(cfg.spacing, 24);
    }

    #[test]
    fn clamp_bounds_offset_range_and_resets_inverted_range() {
        let mut cfg = Config {
            offset_min: -99999,
            offset_max: 99999,
            ..Config::default()
        };
        cfg.clamp();
        assert_eq!(cfg.offset_min, -OFFSET_LIMIT);
        assert_eq!(cfg.offset_max, OFFSET_LIMIT);

        cfg.offset_min = 500;
        cfg.offset_max = 100;
        cfg.clamp();
        let defaults = Config::default();
        assert_eq!(
            (cfg.offset_min, cfg.offset_max),
            (defaults.offset_min, defaults.offset_max)
        );
    }

    #[test]
    fn default_config_roundtrips_through_toml() {
        let cfg = Config::default();
        let toml_str = toml::to_string(&cfg).unwrap();
        let back: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(back, cfg);
    }

    #[test]
    fn saved_config_uses_four_sections_matching_the_gui() {
        let toml_str = toml::to_string(&Config::default()).unwrap();
        for section in ["[position]", "[layout]", "[opacity]", "[colors]"] {
            assert!(
                toml_str.contains(section),
                "missing {section} in:\n{toml_str}"
            );
        }
    }

    #[test]
    fn saved_config_carries_visible_under_layout_section() {
        let cfg = Config {
            visible: false,
            ..Config::default()
        };
        let toml_str = toml::to_string(&cfg).unwrap();
        // The key lives in the [layout] table, next to its sibling filters.
        let layout_start = toml_str.find("[layout]").expect("layout section present");
        let next_section = toml_str[layout_start + 1..]
            .find("\n[")
            .map(|i| layout_start + 1 + i)
            .unwrap_or(toml_str.len());
        assert!(
            toml_str[layout_start..next_section].contains("visible = false"),
            "visible missing from [layout] in:\n{toml_str}"
        );
        // Roundtrip keeps the value...
        let back: Config = toml::from_str(&toml_str).unwrap();
        assert!(!back.visible);
        // ...and a file without the key falls back to visible-by-default.
        let back: Config = toml::from_str("[layout]\nwidth = 400").unwrap();
        assert!(back.visible);
    }

    #[test]
    fn sectioned_toml_roundtrips_changed_fields() {
        let file = "\
[position]
horizontal = \"right\"
anchor = \"bottom\"
offset-x = 40

[layout]
width = 500
talking-only = true

[opacity]
overall = 70
box = 50

[colors]
speaking = \"#00ff00\"
";
        let back: Config = toml::from_str(file).unwrap();
        assert_eq!(back.horizontal, crate::config::HorizontalAnchor::Right);
        assert_eq!(back.anchor, AnchorMode::Bottom);
        assert_eq!(back.offset_x, 40);
        assert_eq!(back.width, 500);
        assert!(back.show_only_talking_users);
        assert_eq!(back.opacity, 70);
        assert_eq!(back.box_opacity, 50);
        assert_eq!(
            back.speaking_color,
            crate::domain::HexColor::from_rgb8(0, 255, 0)
        );
        // Untouched fields keep their defaults.
        let d = Config::default();
        assert_eq!(back.vertical, d.vertical);
        assert_eq!(back.scale, d.scale);
        assert_eq!(back.text_color, d.text_color);

        // And the roundtrip is stable.
        let re: Config = toml::from_str(&toml::to_string(&back).unwrap()).unwrap();
        assert_eq!(re, back);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let back: Config = toml::from_str("[layout]\nwidth = 400").unwrap();
        let expected = Config {
            width: 400,
            ..Config::default()
        };
        assert_eq!(back, expected);
    }

    #[test]
    fn empty_file_parses_to_defaults() {
        let back: Config = toml::from_str("").unwrap();
        assert_eq!(back, Config::default());
    }

    #[test]
    fn alphas_multiply_overall_and_part_percents() {
        let cfg = Config {
            opacity: 50,
            avatar_opacity: 100,
            text_opacity: 50,
            box_opacity: 90,
            ..Config::default()
        };
        let a = cfg.alphas();
        assert_eq!(a.overall, 0.5);
        assert_eq!(a.avatar, 0.5);
        assert_eq!(a.text, 0.25);
        assert_eq!(a.box_bg, 0.45);
    }

    #[test]
    fn default_alphas_leave_everything_but_the_chip_opaque() {
        let a = Config::default().alphas();
        assert_eq!(a.overall, 1.0);
        assert_eq!(a.avatar, 1.0);
        assert_eq!(a.text, 1.0);
        assert_eq!(a.box_bg, 0.9);
    }

    #[test]
    fn speaking_color_falls_back_on_invalid_hex() {
        // Colors are validated at the boundary: an invalid value can't be
        // stored, so a hand-edited file fails to parse and load() falls
        // back to defaults.
        assert!(toml::from_str::<Config>("[colors]\nspeaking = \"not-a-color\"").is_err());
        assert_eq!(
            Config::default().speaking_color,
            crate::domain::HexColor::from_rgb8(0x22, 0xc5, 0x5e)
        );
    }

    #[test]
    fn every_palette_color_roundtrips_through_display() {
        for p in PALETTES {
            let text = p.speaking.to_string();
            assert!(
                text.parse::<crate::domain::HexColor>().is_ok(),
                "palette {}",
                p.name
            );
        }
    }

    #[test]
    fn bounds_contains_is_inclusive_on_both_ends() {
        let b = Bounds { min: 1, max: 3 };
        assert!(b.contains(1));
        assert!(b.contains(2));
        assert!(b.contains(3));
        assert!(!b.contains(0));
        assert!(!b.contains(4));
    }

    #[test]
    fn bounds_clamp_value_pulls_outliers_to_the_nearest_edge() {
        let b = Bounds { min: -10, max: 10 };
        assert_eq!(b.clamp_value(-99), -10);
        assert_eq!(b.clamp_value(0), 0);
        assert_eq!(b.clamp_value(99), 10);
    }

    #[test]
    fn bounds_hint_formats_as_min_max_angle_brackets() {
        assert_eq!(Bounds { min: 200, max: 600 }.hint(), "<200-600>");
    }
}
