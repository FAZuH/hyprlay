//! Field registry: every setting knows its section, label, tooltip, and how
//! to render its control. The sidebar pages and the search results are both
//! projections of this list.

use hyprlay_core::config::AnchorMode;
use hyprlay_core::config::Config;
use hyprlay_core::config::HorizontalAnchor as H;
use hyprlay_core::config::PALETTES;
use hyprlay_core::config::VerticalAnchor as V;
use hyprlay_core::domain::Key;
use hyprlay_core::domain::Value;
use iced::Alignment;
use iced::Border;
use iced::Color;
use iced::Element;
use iced::Font;
use iced::Length;
use iced::font::Weight;
use iced::widget::Column;
use iced::widget::button;
use iced::widget::column;
use iced::widget::container;
use iced::widget::row;
use iced::widget::scrollable;
use iced::widget::scrollable::Scrollbar;
use iced::widget::slider;
use iced::widget::text;
use iced::widget::text_input;
use iced::widget::toggler;
use iced::widget::tooltip;

use super::Gui;
use super::Message;
use super::picker::ColorTarget;
use super::picker::color_editor;
use super::picker::swatch_dot;
use super::theme::ACCENT;
use super::theme::BRIGHT;
use super::theme::FIELD_BG;
use super::theme::MUTED;
use super::theme::plain_style;
use super::theme::scrollbar_style;

const RESET: &str = "↺";

pub(super) const SEARCH_ID: &str = "gui-search";

/// Id of the one-page content scrollable: navigation scrolls it, the
/// scrollspy listens to it, and the measure operation reads its geometry.
pub(super) const CONTENT_SCROLL_ID: &str = "gui-content";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Section {
    Position,
    Layout,
    Opacity,
    Colors,
    Connection,
}

impl Section {
    pub(super) const ALL: [Section; 5] = [
        Section::Position,
        Section::Layout,
        Section::Opacity,
        Section::Colors,
        Section::Connection,
    ];

    pub(super) fn name(self) -> &'static str {
        match self {
            Section::Position => "Position",
            Section::Layout => "Layout",
            Section::Opacity => "Opacity",
            Section::Colors => "Colors",
            Section::Connection => "Connection",
        }
    }

    pub(super) fn at(index: usize) -> Option<Section> {
        Section::ALL.get(index).copied()
    }

    /// Position of this section within [`Section::ALL`].
    pub(super) fn index(self) -> usize {
        Section::ALL
            .iter()
            .position(|s| *s == self)
            .expect("section is in ALL")
    }

    /// The GUI sections Position..Colors and the config/TOML groups are one
    /// and the same concept; this is the bridge for reset commands.
    /// `Connection` is not config at all — its credentials live in
    /// auth.json outside the ctl protocol — so it maps to nothing.
    pub(super) fn group(self) -> Option<hyprlay_core::domain::Group> {
        match self {
            Self::Position => Some(hyprlay_core::domain::Group::Position),
            Self::Layout => Some(hyprlay_core::domain::Group::Layout),
            Self::Opacity => Some(hyprlay_core::domain::Group::Opacity),
            Self::Colors => Some(hyprlay_core::domain::Group::Colors),
            Self::Connection => None,
        }
    }

    /// Widget id of this section header's anchor container — what the
    /// measure operation looks for when mapping layout bounds to sections.
    pub(super) fn anchor_id(self) -> &'static str {
        match self {
            Self::Position => "anchor-position",
            Self::Layout => "anchor-layout",
            Self::Opacity => "anchor-opacity",
            Self::Colors => "anchor-colors",
            Self::Connection => "anchor-connection",
        }
    }
}

pub(super) struct Field {
    pub(super) section: Section,
    pub(super) label: &'static str,
    pub(super) tip: &'static str,
    pub(super) render: fn(&Gui) -> Element<'_, Message>,
}

pub(super) const FIELDS: &[Field] = &[
    Field {
        section: Section::Position,
        label: "corner preset",
        tip: "Snap the overlay to a screen corner. The right side automatically enables right-to-left layout.",
        render: f_presets,
    },
    Field {
        section: Section::Position,
        label: "anchor",
        tip: "Which edge the overlay glues to vertically. Auto follows the position's vertical side, top pins the top edge so rows grow downward, bottom pins the bottom edge so rows grow upward.",
        render: f_anchor,
    },
    Field {
        section: Section::Position,
        label: "right-to-left",
        tip: "Avatar on the right, username to its left, right-aligned. Enabled automatically on right-side presets.",
        render: f_rtl,
    },
    Field {
        section: Section::Position,
        label: "offset slider minimum",
        tip: "Lower bound of the two offset sliders below, in pixels. Lets you reach far-out positions without typing numbers.",
        render: f_offset_min,
    },
    Field {
        section: Section::Position,
        label: "offset slider maximum",
        tip: "Upper bound of the two offset sliders below, in pixels.",
        render: f_offset_max,
    },
    Field {
        section: Section::Position,
        label: "offset x",
        tip: "Horizontal distance in px from the anchored screen edge. Negative values push the other way.",
        render: f_offset_x,
    },
    Field {
        section: Section::Position,
        label: "offset y",
        tip: "Vertical distance in px from the anchored screen edge. Negative values push the other way.",
        render: f_offset_y,
    },
    Field {
        section: Section::Position,
        label: "monitor",
        tip: "Output to show the overlay on. 'active' follows the focused monitor. Changing it restarts the overlay instantly.",
        render: f_monitor,
    },
    Field {
        section: Section::Layout,
        label: "visible",
        tip: "Show or hide the overlay entirely. Hiding collapses it to an empty surface while the daemon keeps running and tracking the channel.",
        render: f_visible,
    },
    Field {
        section: Section::Layout,
        label: "auto-save",
        tip: "Persist every change to config.toml the moment the daemon applies it. Turn off to keep changes session-only until an explicit Save.",
        render: f_auto_save,
    },
    Field {
        section: Section::Layout,
        label: "show over fullscreen",
        tip: "Keep overlay visible when any window is fullscreen. Changing it restarts the overlay instantly. Restart required.",
        render: f_show_on_fullscreen,
    },
    Field {
        section: Section::Layout,
        label: "dim on hover",
        tip: "When on, hovering the overlay dims it to hover opacity for click-through visibility. Hyprland-only, poll every 50 ms.",
        render: f_dim_on_hover,
    },
    Field {
        section: Section::Layout,
        label: "talking-only",
        tip: "Only show participants who are currently speaking.",
        render: f_talking_only,
    },
    Field {
        section: Section::Layout,
        label: "show own user",
        tip: "Include yourself in the overlay.",
        render: f_own_user,
    },
    Field {
        section: Section::Layout,
        label: "width",
        tip: "Panel width in logical pixels.",
        render: f_width,
    },
    Field {
        section: Section::Layout,
        label: "scale",
        tip: "Global scale in percent; multiplies every size (avatar, text, spacing).",
        render: f_scale,
    },
    Field {
        section: Section::Layout,
        label: "avatar size",
        tip: "Avatar diameter in logical pixels.",
        render: f_avatar_size,
    },
    Field {
        section: Section::Layout,
        label: "text size",
        tip: "Username font size in logical pixels.",
        render: f_text_size,
    },
    Field {
        section: Section::Layout,
        label: "spacing",
        tip: "Gap between participant rows in logical pixels.",
        render: f_spacing,
    },
    Field {
        section: Section::Layout,
        label: "max name length",
        tip: "Usernames longer than this are truncated with an ellipsis.",
        render: f_max_name,
    },
    Field {
        section: Section::Opacity,
        label: "overall",
        tip: "Dims everything together: avatars, usernames, badges and the speaking ring.",
        render: f_opacity,
    },
    Field {
        section: Section::Opacity,
        label: "hover opacity",
        tip: "Overall opacity while hovered (0-100). Only used when dim on hover is on.",
        render: f_hover_opacity,
    },
    Field {
        section: Section::Opacity,
        label: "profile picture",
        tip: "Avatar opacity on top of overall.",
        render: f_avatar_opacity,
    },
    Field {
        section: Section::Opacity,
        label: "username text",
        tip: "Username text opacity on top of overall.",
        render: f_text_opacity,
    },
    Field {
        section: Section::Opacity,
        label: "username background",
        tip: "Opacity of the chip behind the username. Set to 0 to hide the chip entirely.",
        render: f_box_opacity,
    },
    Field {
        section: Section::Colors,
        label: "palettes",
        tip: "Color templates that set all three colors at once. Discord is the default look.",
        render: f_palettes,
    },
    Field {
        section: Section::Colors,
        label: "speaking color",
        tip: "Ring color around the avatar while someone talks. Click the swatch to open the picker.",
        render: f_speaking_color,
    },
    Field {
        section: Section::Colors,
        label: "username text color",
        tip: "Username color. Click the swatch to open the picker.",
        render: f_text_color,
    },
    Field {
        section: Section::Colors,
        label: "username background color",
        tip: "Color of the chip behind the username. Click the swatch to open the picker.",
        render: f_box_color,
    },
    Field {
        section: Section::Connection,
        label: "client id",
        tip: "Client ID of your own Discord application, from discord.com/developers/applications. Also register the redirect URI http://127.0.0.1/callback under OAuth2 in the developer portal; Discord requires it even though nothing opens.",
        render: f_auth_client_id,
    },
    Field {
        section: Section::Connection,
        label: "client secret",
        tip: "Client secret of your own Discord application; Apply writes both fields to ~/.config/hyprlay/auth.json (owner-only, never on the ctl socket) and restarts the daemon. Until a complete pair exists the daemon logs credentials_missing and the overlay stays offline.",
        render: f_auth_client_secret,
    },
];

fn f_presets(gui: &Gui) -> Element<'_, Message> {
    let cfg = &gui.config;
    column![
        row![
            preset_button(cfg, H::Left, V::Top, "top-left"),
            preset_button(cfg, H::Right, V::Top, "top-right"),
        ]
        .spacing(8),
        row![
            preset_button(cfg, H::Left, V::Bottom, "bottom-left"),
            preset_button(cfg, H::Right, V::Bottom, "bottom-right"),
        ]
        .spacing(8),
    ]
    .spacing(8)
    .into()
}

pub(super) fn f_rtl(gui: &Gui) -> Element<'_, Message> {
    toggle(gui.config.rtl, |v| Message::SetFlag(Key::Rtl, v))
}

pub(super) fn f_visible(gui: &Gui) -> Element<'_, Message> {
    toggle(gui.config.visible, |v| Message::SetFlag(Key::Visible, v))
}

pub(super) fn f_auto_save(gui: &Gui) -> Element<'_, Message> {
    toggle(gui.config.auto_save, |v| Message::SetFlag(Key::AutoSave, v))
}

pub(super) fn f_offset_min(gui: &Gui) -> Element<'_, Message> {
    number_row(gui, Key::OffsetMin)
}

pub(super) fn f_offset_max(gui: &Gui) -> Element<'_, Message> {
    number_row(gui, Key::OffsetMax)
}

pub(super) fn f_offset_x(gui: &Gui) -> Element<'_, Message> {
    number_row(gui, Key::OffsetX)
}

pub(super) fn f_offset_y(gui: &Gui) -> Element<'_, Message> {
    number_row(gui, Key::OffsetY)
}

pub(super) fn f_monitor(gui: &Gui) -> Element<'_, Message> {
    let mut chips = row![monitor_chip(None, gui.config.monitor.is_none())].spacing(6);
    for name in &gui.monitors {
        chips = chips.push(monitor_chip(
            Some(name.clone()),
            gui.config.monitor.as_deref() == Some(name),
        ));
    }
    chips.into()
}

pub(super) fn f_talking_only(gui: &Gui) -> Element<'_, Message> {
    toggle(gui.config.show_only_talking_users, |v| {
        Message::SetFlag(Key::TalkingOnly, v)
    })
}

pub(super) fn f_own_user(gui: &Gui) -> Element<'_, Message> {
    toggle(gui.config.show_own_user, |v| {
        Message::SetFlag(Key::OwnUser, v)
    })
}

pub(super) fn f_width(gui: &Gui) -> Element<'_, Message> {
    number_row(gui, Key::Width)
}

pub(super) fn f_scale(gui: &Gui) -> Element<'_, Message> {
    number_row(gui, Key::Scale)
}

pub(super) fn f_avatar_size(gui: &Gui) -> Element<'_, Message> {
    number_row(gui, Key::AvatarSize)
}

pub(super) fn f_text_size(gui: &Gui) -> Element<'_, Message> {
    number_row(gui, Key::TextSize)
}

pub(super) fn f_spacing(gui: &Gui) -> Element<'_, Message> {
    number_row(gui, Key::Spacing)
}

pub(super) fn f_max_name(gui: &Gui) -> Element<'_, Message> {
    number_row(gui, Key::MaxName)
}

pub(super) fn f_opacity(gui: &Gui) -> Element<'_, Message> {
    number_row(gui, Key::Opacity)
}

pub(super) fn f_avatar_opacity(gui: &Gui) -> Element<'_, Message> {
    number_row(gui, Key::AvatarOpacity)
}

pub(super) fn f_text_opacity(gui: &Gui) -> Element<'_, Message> {
    number_row(gui, Key::TextOpacity)
}

pub(super) fn f_box_opacity(gui: &Gui) -> Element<'_, Message> {
    number_row(gui, Key::BoxOpacity)
}

pub(super) fn f_show_on_fullscreen(gui: &Gui) -> Element<'_, Message> {
    toggle(gui.config.show_on_fullscreen, |v| {
        Message::SetFlag(Key::ShowOnFullscreen, v)
    })
}

pub(super) fn f_dim_on_hover(gui: &Gui) -> Element<'_, Message> {
    toggle(gui.config.dim_on_hover, |v| {
        Message::SetFlag(Key::DimOnHover, v)
    })
}

pub(super) fn f_hover_opacity(gui: &Gui) -> Element<'_, Message> {
    let content = number_row(gui, Key::HoverOpacity);
    if gui.config.dim_on_hover {
        content
    } else {
        container(
            column![
                content,
                text("requires dim on hover to take effect")
                    .size(10)
                    .color(MUTED)
            ]
            .spacing(4),
        )
        .style(|_theme| container::Style {
            text_color: Some(MUTED),
            ..container::Style::default()
        })
        .into()
    }
}

pub(super) fn f_palettes(_gui: &Gui) -> Element<'_, Message> {
    let mut chips = row![].spacing(6);
    for (i, p) in PALETTES.iter().enumerate() {
        chips = chips.push(
            button(
                row![
                    text(p.name.to_string()).size(12).color(BRIGHT),
                    swatch_dot(p.speaking),
                    swatch_dot(p.text),
                    swatch_dot(p.box_bg),
                ]
                .spacing(5)
                .align_y(Alignment::Center),
            )
            .on_press(Message::Palette(i))
            .style(|_t, _s| button::Style {
                background: Some(FIELD_BG.into()),
                border: Border {
                    radius: 6.0.into(),
                    ..Border::default()
                },
                ..button::Style::default()
            })
            .padding([4, 8]),
        );
    }
    chips.into()
}

pub(super) fn f_speaking_color(gui: &Gui) -> Element<'_, Message> {
    color_editor(gui, ColorTarget::Speaking)
}

pub(super) fn f_text_color(gui: &Gui) -> Element<'_, Message> {
    color_editor(gui, ColorTarget::Text)
}

pub(super) fn f_box_color(gui: &Gui) -> Element<'_, Message> {
    color_editor(gui, ColorTarget::Box)
}

pub(super) fn f_auth_client_id(gui: &Gui) -> Element<'_, Message> {
    text_input("", &gui.auth_client_id)
        .on_input(Message::AuthClientId)
        .size(12)
        .padding([3, 6])
        .width(Length::Fill)
        .into()
}

pub(super) fn f_auth_client_secret(gui: &Gui) -> Element<'_, Message> {
    text_input("", &gui.auth_client_secret)
        .on_input(Message::AuthClientSecret)
        // Masked so a screen share or shoulder-surf never exposes it.
        .secure(true)
        .size(12)
        .padding([3, 6])
        .width(Length::Fill)
        .into()
}

/// Connection credentials commit as one pair: apply writes both drafts to
/// auth.json (an incomplete pair clears the file) and restarts the daemon.
fn auth_apply_button() -> Element<'static, Message> {
    button(text("apply connection").size(12))
        .on_press(Message::AuthApply)
        .padding([5, 12])
        .style(plain_style())
        .into()
}

/// Every section on one scrollable page. The sidebar buttons and
/// Ctrl+1..5 are anchors into this page: the outer scrollable carries
/// [`CONTENT_SCROLL_ID`] (the jump target) and reports its viewport
/// offset through [`Message::Scrolled`] (the scrollspy), while each
/// header sits in a container tagged with [`Section::anchor_id`] for the
/// measure operation to find.
pub(super) fn settings_page(gui: &Gui) -> Element<'_, Message> {
    let mut sections = column![].spacing(24);
    for section in Section::ALL {
        sections = sections.push(section_block(gui, section));
    }
    scroll_page(sections)
        .id(iced::widget::Id::new(CONTENT_SCROLL_ID))
        .on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y))
        .into()
}

pub(super) fn search_page(gui: &Gui) -> Element<'_, Message> {
    let query = gui.search.trim();
    let mut col = column![text(format!("Search “{query}”")).size(15).color(BRIGHT)].spacing(12);
    let mut hits = 0;
    for field in FIELDS.iter().filter(|f| search_matches(f, query)) {
        hits += 1;
        col = col.push(
            column![
                text(field.section.name().to_string()).size(10).color(MUTED),
                field_row(gui, field),
            ]
            .spacing(2),
        );
    }
    if hits == 0 {
        col = col.push(text("no settings match").color(MUTED));
    }
    scroll_page(col).into()
}

/// Search covers the label, the tooltip text, and the section name — so
/// "dim", "avatar", "click" all find what the user means.
pub(super) fn search_matches(field: &Field, query: &str) -> bool {
    let q = query.to_lowercase();
    !q.is_empty()
        && (field.label.to_lowercase().contains(&q)
            || field.tip.to_lowercase().contains(&q)
            || field.section.name().to_lowercase().contains(&q))
}

fn section_header(section: Section) -> Element<'static, Message> {
    let mut header = row![
        text(section.name().to_string())
            .size(18)
            .font(Font {
                weight: Weight::Bold,
                ..Font::default()
            })
            .color(BRIGHT),
        iced::widget::Space::new().width(Length::Fill),
    ];
    // No config group means there is no per-section default to restore,
    // so the reset control would be a lie — hide it.
    if section.group().is_some() {
        header = header.push(
            button(text("reset section").size(11))
                .on_press(Message::ResetSection(section))
                .style(plain_style()),
        );
    }
    let header_row = header.spacing(8).align_y(Alignment::Center);
    column![
        header_row,
        container(iced::widget::Space::new().height(Length::Fixed(1.0)))
            .width(Length::Fill)
            .style(|_t| container::Style {
                background: Some(Color::from_rgba(0.50, 0.51, 0.55, 0.18).into()),
                ..container::Style::default()
            })
    ]
    .spacing(8)
    .into()
}

/// One section's header (with its per-section reset — or, for Connection,
/// the apply button) plus its fields, as rendered on the one-page view.
fn section_block(gui: &Gui, section: Section) -> Element<'_, Message> {
    let mut col = column![section_anchor(section)].spacing(12);
    for field in FIELDS.iter().filter(|f| f.section == section) {
        col = col.push(field_row(gui, field));
    }
    // Sections without a config group (Connection) have nothing to reset;
    // their apply button commits the credential drafts instead.
    if section.group().is_none() {
        col = col.push(auth_apply_button());
    }
    col.into()
}

/// The section header wrapped in an anchor container: the container's
/// widget id is what the measure operation records the header's offset
/// from, and the jump scrolls exactly to it.
fn section_anchor(section: Section) -> Element<'static, Message> {
    container(section_header(section))
        .id(iced::widget::Id::new(section.anchor_id()))
        .width(Length::Fill)
        .into()
}

fn field_row<'a>(gui: &'a Gui, field: &Field) -> Element<'a, Message> {
    column![tip_label(field.label), (field.render)(gui),]
        .spacing(4)
        .into()
}

/// Field label with a hover tooltip.
fn tip_label(label: &str) -> Element<'static, Message> {
    tooltip(
        text(format!("{label}:")).size(12).color(MUTED),
        text(label_tip_lookup(label)).size(11).color(BRIGHT),
        tooltip::Position::FollowCursor,
    )
    .padding(6)
    .style(|_t| container::Style {
        background: Some(Color::from_rgb(0.09, 0.09, 0.11).into()),
        border: Border {
            color: Color::from_rgb(0.25, 0.26, 0.30),
            radius: 4.0.into(),
            width: 1.0,
        },
        text_color: Some(BRIGHT),
        ..container::Style::default()
    })
    .into()
}

/// Tooltips live in the field registry; look the tip up by label so the
/// label element stays cheap to build.
fn label_tip_lookup(label: &str) -> &'static str {
    FIELDS
        .iter()
        .find(|f| f.label == label)
        .map(|f| f.tip)
        .unwrap_or("")
}

/// The dressed scrollable every page rides on: padding, scrollbar, style,
/// and fill height. Callers attach what makes the page addressable before
/// `.into()` — the one-pager adds [`CONTENT_SCROLL_ID`] and the
/// [`Message::Scrolled`] hook; the search page needs neither.
fn scroll_page(content: Column<'_, Message>) -> iced::widget::Scrollable<'_, Message> {
    let page_padding = iced::Padding {
        top: 8.0,
        right: 16.0,
        bottom: 16.0,
        left: 16.0,
    };
    scrollable(container(content).padding(page_padding).width(Length::Fill))
        .direction(scrollable::Direction::Vertical(
            Scrollbar::new().width(8.0).scroller_width(8.0).margin(4.0),
        ))
        .style(scrollbar_style)
        .height(Length::Fill)
}

fn toggle(is_on: bool, on_toggle: impl Fn(bool) -> Message + 'static) -> Element<'static, Message> {
    row![toggler(is_on).on_toggle(on_toggle)].into()
}

/// One numeric knob: slider (when the field has an envelope) + integer text
/// input + reset-to-default. Typing goes through [`Message::NumText`]; the
/// draft keeps half-typed or out-of-range text from snapping back.
fn number_row(gui: &Gui, key: Key) -> Element<'static, Message> {
    let value = key.value_of(&gui.config);
    let Value::Num(value) = value else {
        unreachable!("number_row only renders numeric keys");
    };
    let shown = gui
        .num_drafts
        .get(&key)
        .cloned()
        .unwrap_or_else(|| value.to_string());
    let input = |w: f32| {
        text_input("", &shown)
            .on_input(move |s| Message::NumText(key, s))
            .size(12)
            .width(Length::Fixed(w))
            .padding([3, 6])
    };
    match key.slider_bounds(&gui.config) {
        Some((min, max)) => row![
            slider(min..=max, value as f32, move |v| Message::NumDrag(key, v)).width(Length::Fill),
            input(72.0),
            reset_button(Message::NumReset(key)),
        ]
        .spacing(8)
        .into(),
        None => row![input(96.0), reset_button(Message::NumReset(key)),]
            .spacing(8)
            .into(),
    }
}

pub(super) fn reset_button(reset: Message) -> Element<'static, Message> {
    button(text(RESET).size(12))
        .on_press(reset)
        .padding([2, 8])
        .style(|_t, _s| button::Style {
            background: Some(FIELD_BG.into()),
            text_color: Color::from_rgb(0.6, 0.62, 0.66),
            ..button::Style::default()
        })
        .into()
}

fn monitor_chip(name: Option<String>, selected: bool) -> Element<'static, Message> {
    let label = name.clone().unwrap_or_else(|| "active".to_string());
    let bg = if selected { ACCENT } else { FIELD_BG };
    button(text(label))
        .on_press(Message::SwitchMonitor(name))
        .style(move |_t, _s| button::Style {
            background: Some(bg.into()),
            text_color: Color::WHITE,
            ..button::Style::default()
        })
        .padding([4, 10])
        .into()
}

/// Tri-state glue-edge selector: auto | top | bottom as chips, mirroring
/// the monitor chip pattern (selected state highlighted).
pub(super) fn f_anchor(gui: &Gui) -> Element<'_, Message> {
    let modes = [
        (AnchorMode::Auto, "auto"),
        (AnchorMode::Top, "top"),
        (AnchorMode::Bottom, "bottom"),
    ];
    let mut chips = row![].spacing(6);
    for (mode, label) in modes {
        let selected = gui.config.anchor == mode;
        chips = chips.push(anchor_chip(mode, label, selected));
    }
    chips.into()
}

fn anchor_chip(mode: AnchorMode, label: &str, selected: bool) -> Element<'static, Message> {
    let bg = if selected { ACCENT } else { FIELD_BG };
    button(text(label.to_string()))
        .on_press(Message::Anchor(mode))
        .style(move |_t, _s| button::Style {
            background: Some(bg.into()),
            text_color: Color::WHITE,
            ..button::Style::default()
        })
        .padding([4, 10])
        .into()
}

fn preset_button<'a>(cfg: &'a Config, h: H, v: V, label: &'a str) -> Element<'a, Message> {
    let selected = cfg.horizontal == h && cfg.vertical == v;
    let base_bg = if selected { ACCENT } else { FIELD_BG };
    button(text(label.to_string()))
        .on_press(Message::Position(h, v))
        .style(move |_theme, _status| button::Style {
            background: Some(base_bg.into()),
            text_color: Color::WHITE,
            ..button::Style::default()
        })
        .padding([6, 12])
        .width(Length::Fill)
        .into()
}
