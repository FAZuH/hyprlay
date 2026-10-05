//! Field registry: every setting knows its section, label, tooltip, and how
//! to render its control. The sidebar pages and the search results are both
//! projections of this list.

use hyprlay_core::config::AnchorMode;
use hyprlay_core::config::Config;
use hyprlay_core::config::HorizontalAnchor as H;
use hyprlay_core::config::PALETTES;
use hyprlay_core::config::RosterOrder;
use hyprlay_core::config::VerticalAnchor as V;
use hyprlay_core::domain::Key;
use hyprlay_core::domain::MonitorTarget;
use hyprlay_core::domain::Value;
use hyprlay_core::domain::corner_of;
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

use super::Credential;
use super::FocusTarget;
use super::Gui;
use super::Message;
use super::picker::ColorTarget;
use super::picker::color_editor;
use super::picker::swatch_dot;
use super::theme::ACCENT;
use super::theme::BRIGHT;
use super::theme::FIELD_BG;
use super::theme::MUTED;
use super::theme::focus_fill;
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

impl Credential {
    /// Widget id of the row container. The reveal operation reads a focused
    /// row's geometry off exactly this, keyed rows and credential rows alike.
    fn row_id(self) -> &'static str {
        match self {
            Self::ClientId => "row-client-id",
            Self::ClientSecret => "row-client-secret",
        }
    }

    /// Widget id of the row's text input: what `operation::focus` hands typing
    /// to, and what a mouse click lands on. Separate from `row_id` because two
    /// widgets in one tree cannot share an id.
    pub(super) fn input_id(self) -> &'static str {
        match self {
            Self::ClientId => "input-client-id",
            Self::ClientSecret => "input-client-secret",
        }
    }
}

pub(super) struct Field {
    pub(super) section: Section,
    pub(super) label: &'static str,
    pub(super) tip: &'static str,
    /// The config key this row edits, or `None` for a row that edits none.
    ///
    /// `None` splits two ways, and `credential_of` tells them apart: the
    /// palettes row is mouse-only, while the two credential rows are
    /// keyboard-reachable under a [`Credential`] of their own — they edit
    /// auth.json, which is no config key. A `Some` key must always name a
    /// `Key` a reset could replay, so no credential may ever carry one.
    pub(super) key: Option<Key>,
    pub(super) render: fn(&Gui) -> Element<'_, Message>,
}

pub(super) const FIELDS: &[Field] = &[
    Field {
        section: Section::Position,
        label: "corner preset",
        tip: "Snap the overlay to a screen corner. The right side automatically enables right-to-left layout.",
        key: Some(Key::Position),
        render: f_presets,
    },
    Field {
        section: Section::Position,
        label: "anchor",
        tip: "Which edge the overlay glues to vertically. Auto follows the position's vertical side, top pins the top edge so rows grow downward, bottom pins the bottom edge so rows grow upward.",
        key: Some(Key::Anchor),
        render: f_anchor,
    },
    Field {
        section: Section::Position,
        label: "right-to-left",
        tip: "Avatar on the right, username to its left, right-aligned. Enabled automatically on right-side presets.",
        key: Some(Key::Rtl),
        render: f_rtl,
    },
    Field {
        section: Section::Position,
        label: "offset slider minimum",
        tip: "Lower bound of the two offset sliders below, in pixels. Lets you reach far-out positions without typing numbers.",
        key: Some(Key::OffsetMin),
        render: f_offset_min,
    },
    Field {
        section: Section::Position,
        label: "offset slider maximum",
        tip: "Upper bound of the two offset sliders below, in pixels.",
        key: Some(Key::OffsetMax),
        render: f_offset_max,
    },
    Field {
        section: Section::Position,
        label: "offset x",
        tip: "Horizontal distance in px from the anchored screen edge. Negative values push the other way.",
        key: Some(Key::OffsetX),
        render: f_offset_x,
    },
    Field {
        section: Section::Position,
        label: "offset y",
        tip: "Vertical distance in px from the anchored screen edge. Negative values push the other way.",
        key: Some(Key::OffsetY),
        render: f_offset_y,
    },
    Field {
        section: Section::Position,
        label: "monitor",
        tip: "Output to show the overlay on. 'active' follows the focused monitor. Changing it restarts the overlay instantly.",
        key: Some(Key::Monitor),
        render: f_monitor,
    },
    Field {
        section: Section::Layout,
        label: "visible",
        tip: "Show or hide the overlay entirely. Hiding collapses it to an empty surface while the daemon keeps running and tracking the channel.",
        key: Some(Key::Visible),
        render: f_visible,
    },
    Field {
        section: Section::Layout,
        label: "auto-save",
        tip: "Persist every change to config.toml the moment the daemon applies it. Turn off to keep changes session-only until an explicit Save.",
        key: Some(Key::AutoSave),
        render: f_auto_save,
    },
    Field {
        section: Section::Layout,
        label: "show over fullscreen",
        tip: "Keep overlay visible when any window is fullscreen. Changing it restarts the overlay instantly. Restart required.",
        key: Some(Key::ShowOnFullscreen),
        render: f_show_on_fullscreen,
    },
    Field {
        section: Section::Layout,
        label: "dim on hover",
        tip: "When on, hovering the overlay dims it to hover opacity for click-through visibility. Hyprland-only, poll every 50 ms.",
        key: Some(Key::DimOnHover),
        render: f_dim_on_hover,
    },
    Field {
        section: Section::Layout,
        label: "talking-only",
        tip: "Only show participants who are currently speaking.",
        key: Some(Key::TalkingOnly),
        render: f_talking_only,
    },
    Field {
        section: Section::Layout,
        label: "show own user",
        tip: "Include yourself in the overlay.",
        key: Some(Key::OwnUser),
        render: f_own_user,
    },
    Field {
        section: Section::Layout,
        label: "roster order",
        tip: "How participants are ordered. Join order keeps Discord's arrival order, name sorts alphabetically, recent speakers bubble the last person who talked to the top.",
        key: Some(Key::RosterOrder),
        render: f_roster_order,
    },
    Field {
        section: Section::Layout,
        label: "width",
        tip: "Panel width in logical pixels.",
        key: Some(Key::Width),
        render: f_width,
    },
    Field {
        section: Section::Layout,
        label: "scale",
        tip: "Global scale in percent; multiplies every size (avatar, text, spacing).",
        key: Some(Key::Scale),
        render: f_scale,
    },
    Field {
        section: Section::Layout,
        label: "avatar size",
        tip: "Avatar diameter in logical pixels.",
        key: Some(Key::AvatarSize),
        render: f_avatar_size,
    },
    Field {
        section: Section::Layout,
        label: "text size",
        tip: "Username font size in logical pixels.",
        key: Some(Key::TextSize),
        render: f_text_size,
    },
    Field {
        section: Section::Layout,
        label: "spacing",
        tip: "Gap between participant rows in logical pixels.",
        key: Some(Key::Spacing),
        render: f_spacing,
    },
    Field {
        section: Section::Layout,
        label: "max name length",
        tip: "Usernames longer than this are truncated with an ellipsis.",
        key: Some(Key::MaxName),
        render: f_max_name,
    },
    Field {
        section: Section::Layout,
        label: "max rows",
        tip: "Cap how many participant rows render. Overflow rows hide behind a +N pill; 0 shows everyone.",
        key: Some(Key::MaxRows),
        render: f_max_rows,
    },
    Field {
        section: Section::Opacity,
        label: "overall",
        tip: "Dims everything together: avatars, usernames, glyphs and the speaking ring.",
        key: Some(Key::Opacity),
        render: f_opacity,
    },
    Field {
        section: Section::Opacity,
        label: "hover opacity",
        tip: "Overall opacity while hovered (0-100). Only used when dim on hover is on.",
        key: Some(Key::HoverOpacity),
        render: f_hover_opacity,
    },
    Field {
        section: Section::Opacity,
        label: "profile picture",
        tip: "Avatar opacity on top of overall.",
        key: Some(Key::AvatarOpacity),
        render: f_avatar_opacity,
    },
    Field {
        section: Section::Opacity,
        label: "username text",
        tip: "Username text opacity on top of overall.",
        key: Some(Key::TextOpacity),
        render: f_text_opacity,
    },
    Field {
        section: Section::Opacity,
        label: "username background",
        tip: "Opacity of the chip behind the username. Set to 0 to hide the chip entirely.",
        key: Some(Key::BoxOpacity),
        render: f_box_opacity,
    },
    Field {
        section: Section::Colors,
        label: "palettes",
        tip: "Color templates that set all three colors at once. Discord is the default look.",
        key: None,
        render: f_palettes,
    },
    Field {
        section: Section::Colors,
        label: "speaking color",
        tip: "Ring color around the avatar while someone talks. Click the swatch to open the picker.",
        key: Some(Key::SpeakingColor),
        render: f_speaking_color,
    },
    Field {
        section: Section::Colors,
        label: "username text color",
        tip: "Username color. Click the swatch to open the picker.",
        key: Some(Key::TextColor),
        render: f_text_color,
    },
    Field {
        section: Section::Colors,
        label: "username background color",
        tip: "Color of the chip behind the username. Click the swatch to open the picker.",
        key: Some(Key::BoxColor),
        render: f_box_color,
    },
    Field {
        section: Section::Connection,
        label: "client id",
        tip: "Client ID of your own Discord application, from discord.com/developers/applications. Also register the redirect URI http://127.0.0.1/callback under OAuth2 in the developer portal; Discord requires it even though nothing opens.",
        key: None,
        render: f_auth_client_id,
    },
    Field {
        section: Section::Connection,
        label: "client secret",
        tip: "Client secret of your own Discord application; Apply writes both fields to ~/.config/hyprlay/auth.json (owner-only, never on the ctl socket) and restarts the daemon. Until a complete pair exists the daemon logs credentials_missing and the overlay stays offline.",
        key: None,
        render: f_auth_client_secret,
    },
];

fn f_presets(gui: &Gui) -> Element<'_, Message> {
    let cfg = &gui.config;
    // Two per row, so the grid reads left to right: the keyboard steps
    // through `PRESETS` in this same order, and the two cannot drift because
    // there is only one list.
    let [top_left, top_right, bottom_left, bottom_right] = PRESETS;
    column![
        row![preset_button(cfg, top_left), preset_button(cfg, top_right),].spacing(8),
        row![
            preset_button(cfg, bottom_left),
            preset_button(cfg, bottom_right),
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
    let current = Key::Monitor.value_of(&gui.config);
    let mut chips = row![].spacing(6);
    for value in options(gui, Key::Monitor) {
        let selected = value == current;
        chips = chips.push(monitor_chip(value, selected));
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

/// Tri-state roster-order selector: join-order | name | recent-speakers as
/// chips, mirroring the anchor chip pattern (selected state highlighted).
pub(super) fn f_roster_order(gui: &Gui) -> Element<'_, Message> {
    let mut chips = row![].spacing(6);
    for order in ROSTER_ORDERS {
        let selected = gui.config.roster_order == order;
        chips = chips.push(roster_order_chip(order, order.as_str(), selected));
    }
    chips.into()
}

fn roster_order_chip(mode: RosterOrder, label: &str, selected: bool) -> Element<'static, Message> {
    let bg = if selected { ACCENT } else { FIELD_BG };
    button(text(label.to_string()))
        .on_press(Message::RosterOrder(mode))
        .style(move |_t, _s| button::Style {
            background: Some(bg.into()),
            text_color: Color::WHITE,
            ..button::Style::default()
        })
        .padding([4, 10])
        .into()
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

pub(super) fn f_max_rows(gui: &Gui) -> Element<'_, Message> {
    number_row(gui, Key::MaxRows)
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
        // The id `FocusTarget::Credential` hands typing to; without it iced
        // cannot focus a text input on request (see `move_focus`).
        .id(Credential::ClientId.input_id())
        .on_input(Message::AuthClientId)
        .size(12)
        .padding([3, 6])
        .width(Length::Fill)
        .into()
}

pub(super) fn f_auth_client_secret(gui: &Gui) -> Element<'_, Message> {
    text_input("", &gui.auth_client_secret)
        .id(Credential::ClientSecret.input_id())
        .on_input(Message::AuthClientSecret)
        // Masked so a screen share or shoulder-surf never exposes it. Focus
        // changes nothing here: the masking is the widget's, not a state the
        // focus target owns, so landing on the row cannot reveal it.
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
        .style(plain_style(false))
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
        // R-27: the empty state names why it is empty and the one action
        // that fills it. `search_matches` covers labels, tooltips, and
        // section names, so saying so is the difference between a dead end
        // and a hint.
        col = col.push(
            text(format!(
                "no settings match \u{201c}{query}\u{201d}. Search covers labels, tooltips, and section names."
            ))
            .color(MUTED),
        );
    }
    // The same id the one-pager carries: the reveal operation matches on it,
    // so without it `Jump::Field` finds no scrollable and yields nothing, and
    // Tab on the search page rings a row without bringing it into view.
    scroll_page(col)
        .id(iced::widget::Id::new(CONTENT_SCROLL_ID))
        .into()
}

/// The credential rows, in render order, keyed by the label that names them in
/// [`FIELDS`]. The label is this registry's own row identity — `label_tip_lookup`
/// already keys on it — so a row is a credential exactly when its label says
/// so, and the two lists cannot drift without a test failing.
const CREDENTIAL_ROWS: [(Credential, &str); 2] = [
    (Credential::ClientId, "client id"),
    (Credential::ClientSecret, "client secret"),
];

/// The credential this row is, or `None` for every other row (including the
/// palettes row, which is keyless *and* mouse-only).
fn credential_of(field: &Field) -> Option<Credential> {
    CREDENTIAL_ROWS
        .iter()
        .find(|(_, label)| *label == field.label)
        .map(|(credential, _)| *credential)
}

/// What keyboard focus holds for this row, or `None` for a row only the mouse
/// reaches. A keyed row is named by its config `Key` — the identity the rest of
/// the app reasons in — and a credential row names itself, because it has no key
/// and must never be given one.
fn focus_target_of(field: &Field) -> Option<FocusTarget> {
    match field.key {
        Some(key) => Some(FocusTarget::Field(key)),
        None => credential_of(field).map(FocusTarget::Credential),
    }
}

/// The widget id a focused row is tagged with, so the reveal operation can
/// measure it. `None` for chrome: those are not rows and never need a reveal.
pub(super) fn row_id(target: FocusTarget) -> Option<iced::widget::Id> {
    match target {
        FocusTarget::Field(key) => Some(iced::widget::Id::new(key.name())),
        FocusTarget::Credential(credential) => Some(iced::widget::Id::new(credential.row_id())),
        _ => None,
    }
}

/// The focused rows the current page renders, in render order: every row the
/// keyboard can land on when the one-pager is up, only the search hits on the
/// search page. The tab order walks exactly these, so Tab cannot land on a row
/// that is not in the tree — and because it is `FIELDS` order, the order *is*
/// the visual order.
///
/// The query is trimmed here because `view` picks the page on
/// `gui.search.trim()` and `search_page` filters with the trimmed query: a
/// trailing space must narrow neither the page nor the tab order, or Tab skips
/// every field row.
pub(super) fn rendered_targets(query: &str) -> impl Iterator<Item = FocusTarget> + '_ {
    let query = query.trim();
    FIELDS
        .iter()
        .filter(move |f| query.is_empty() || search_matches(f, query))
        .filter_map(focus_target_of)
}

/// The section a focused row is declared under — the sidebar entry that owns
/// it, and so the section a keyboard user is in once focus lands on it.
/// `None` for chrome, and for a target no row declares.
pub(super) fn section_of(target: FocusTarget) -> Option<Section> {
    FIELDS
        .iter()
        .find(|f| focus_target_of(f) == Some(target))
        .map(|f| f.section)
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
                .style(plain_style(false)),
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
    // The row is the focus target: a fill behind label + control, because only
    // this one place renders every field row. Keyed and credential rows go
    // through the same two lines, so one indicator covers both and neither can
    // drift from the other.
    let focused = focus_target_of(field).is_some_and(|t| gui.focus == Some(t));
    let mut row =
        container(column![tip_label(field.label, focused), (field.render)(gui)].spacing(4))
            .width(Length::Fill)
            .style(focus_fill(focused));
    if let Some(id) = focus_target_of(field).and_then(row_id) {
        row = row.id(id);
    }
    row.into()
}

/// Field label with a hover tooltip.
///
/// The label lifts to `BRIGHT` on a focused row: that row is filled, and
/// `MUTED` cannot hold the audited contrast against a fill light enough to
/// read as an indicator (see `theme::FOCUS_FILL`).
fn tip_label(label: &str, focused: bool) -> Element<'static, Message> {
    tooltip(
        text(format!("{label}:"))
            .size(12)
            .color(if focused { BRIGHT } else { MUTED }),
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
/// `.into()` — both pages add [`CONTENT_SCROLL_ID`], the one that also adds
/// the [`Message::Scrolled`] hook.
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

/// How far one arrow press moves a number row. The row's slider steps by this
/// too (see [`number_row`]), so the keyboard and the mouse walk the row the
/// same distance and there is one step per row rather than two.
pub(super) const NUM_STEP: i64 = 1;

/// Widget id of a number row's integer input: what `operation::focus` hands
/// typing to when Enter moves the keyboard into the row. Separate from the row
/// container's id because two widgets in one tree cannot share an id.
pub(super) fn num_input_id(key: Key) -> iced::widget::Id {
    iced::widget::Id::from(format!("num-input-{}", key.name()))
}

/// One numeric knob: slider (when the field has an envelope) + integer text
/// input + reset-to-default. Typing goes through [`Message::NumText`] and
/// commits on [`Message::NumSubmit`]; the draft keeps half-typed or
/// out-of-range text from snapping back.
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
            // The id the keyboard hands typing to when Enter moves into this
            // row; without it iced cannot focus a text input on request.
            .id(num_input_id(key))
            .on_input(move |s| Message::NumText(key, s))
            .on_submit(Message::NumSubmit(key))
            .size(12)
            .width(Length::Fixed(w))
            .padding([3, 6])
    };
    match key.slider_bounds(&gui.config) {
        Some((min, max)) => row![
            slider(min..=max, value as f32, move |v| Message::NumDrag(key, v))
                // The slider's own step, written down: the arrow keys step the
                // row by [`NUM_STEP`], so the two agree by construction.
                .step(NUM_STEP as f32)
                .width(Length::Fill),
            input(72.0),
            reset_button(Message::ResetFocused(key)),
        ]
        .spacing(8)
        .into(),
        None => row![input(96.0), reset_button(Message::ResetFocused(key)),]
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

fn monitor_chip(value: Value, selected: bool) -> Element<'static, Message> {
    let name = monitor_name(&value);
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

/// The output a monitor option stands for, as the value [`Message::SwitchMonitor`]
/// carries: `None` is the focused monitor, `Some` a named one. Also how the
/// chip labels itself, so the label and the target cannot name different things.
pub(super) fn monitor_name(value: &Value) -> Option<String> {
    match value {
        Value::Target(MonitorTarget::Active) => None,
        Value::Target(MonitorTarget::Named(name)) => Some(name.clone()),
        other => unreachable!("the monitor row only offers targets, not {other:?}"),
    }
}

/// Tri-state glue-edge selector: auto | top | bottom as chips, mirroring
/// the monitor chip pattern (selected state highlighted).
pub(super) fn f_anchor(gui: &Gui) -> Element<'_, Message> {
    let mut chips = row![].spacing(6);
    for mode in ANCHORS {
        let selected = gui.config.anchor == mode;
        chips = chips.push(anchor_chip(mode, mode.as_str(), selected));
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

fn preset_button<'a>(cfg: &'a Config, (h, v, label): (H, V, &'static str)) -> Element<'a, Message> {
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

/// The corner presets, in reading order: top-left, top-right, bottom-left,
/// bottom-right. `f_presets` paints two per row in this order and
/// [`options`] steps through it in this order, so what Right moves to is what
/// the eye reads next.
const PRESETS: [(H, V, &str); 4] = [
    (H::Left, V::Top, "top-left"),
    (H::Right, V::Top, "top-right"),
    (H::Left, V::Bottom, "bottom-left"),
    (H::Right, V::Bottom, "bottom-right"),
];

/// The vertical glue edges, in chip order: auto, top, bottom.
const ANCHORS: [AnchorMode; 3] = [AnchorMode::Auto, AnchorMode::Top, AnchorMode::Bottom];

/// The roster orderings, in chip order: join-order, name, recent-speakers.
const ROSTER_ORDERS: [RosterOrder; 3] = [
    RosterOrder::JoinOrder,
    RosterOrder::Name,
    RosterOrder::RecentSpeakers,
];

/// The options one row offers, in the order the row renders them — the order
/// Enter and Right step forward through and Left steps back through, so the
/// keyboard moves between choices exactly as they sit on screen.
///
/// Empty for a row with no fixed set of choices: the number rows, the colour
/// editors, the togglers. That is what makes a step on one of those inert
/// rather than a guess. `palettes` is absent on purpose — it is keyless and
/// mouse-only, and it writes three keys at once instead of offering a choice
/// between them.
pub(super) fn options(gui: &Gui, key: Key) -> Vec<Value> {
    match key {
        Key::Position => PRESETS
            .iter()
            .map(|(h, v, _)| Value::Corner(corner_of(*h, *v)))
            .collect(),
        Key::Anchor => ANCHORS.iter().map(|mode| Value::Anchor(*mode)).collect(),
        Key::RosterOrder => ROSTER_ORDERS
            .iter()
            .map(|order| Value::RosterOrder(*order))
            .collect(),
        // "active" first, then every output the compositor reported, which is
        // the order `f_monitor` paints the chips in.
        Key::Monitor => std::iter::once(Value::Target(MonitorTarget::Active))
            .chain(
                gui.monitors
                    .iter()
                    .map(|name| Value::Target(MonitorTarget::Named(name.clone()))),
            )
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use hyprlay_core::domain::Corner;
    use hyprlay_core::domain::MonitorTarget;

    use super::*;

    /// The keyboard order has to be the visual order, or Right moves to a chip
    /// the row does not show next. Every option-select row's list is read off
    /// the very table its renderer paints from, so these assertions are about
    /// what that table says, not about two lists agreeing by luck.
    #[test]
    fn the_options_are_in_the_rows_visual_order() {
        let mut gui = crate::gui::test_gui("");
        assert_eq!(
            options(&gui, Key::Position),
            [
                Value::Corner(Corner::TopLeft),
                Value::Corner(Corner::TopRight),
                Value::Corner(Corner::BottomLeft),
                Value::Corner(Corner::BottomRight),
            ],
            "the presets read left to right, then down, as the 2x2 grid paints them"
        );
        assert_eq!(
            options(&gui, Key::Anchor),
            [
                Value::Anchor(AnchorMode::Auto),
                Value::Anchor(AnchorMode::Top),
                Value::Anchor(AnchorMode::Bottom),
            ],
            "the anchor chips paint in this order"
        );
        assert_eq!(
            options(&gui, Key::RosterOrder),
            [
                Value::RosterOrder(RosterOrder::JoinOrder),
                Value::RosterOrder(RosterOrder::Name),
                Value::RosterOrder(RosterOrder::RecentSpeakers),
            ],
            "and so do the roster-order chips"
        );

        // "active" leads the monitor row, then every output the compositor
        // reported — the order `f_monitor` paints them in.
        gui.monitors = ["DP-1".to_string(), "HDMI-A-1".to_string()].into();
        assert_eq!(
            options(&gui, Key::Monitor),
            [
                Value::Target(MonitorTarget::Active),
                Value::Target(MonitorTarget::Named("DP-1".into())),
                Value::Target(MonitorTarget::Named("HDMI-A-1".into())),
            ],
            "the monitor row leads with the focused output"
        );
    }

    /// Every option must be distinct, or a step lands on a value the row
    /// cannot tell apart from the one it left: `step_command` finds the
    /// current option by value, so a duplicate would make two chips the same
    /// choice and one of them unsteppable.
    #[test]
    fn no_row_offers_the_same_option_twice() {
        let mut gui = crate::gui::test_gui("");
        gui.monitors = ["DP-1".to_string(), "HDMI-A-1".to_string()].into();
        for key in [Key::Position, Key::Anchor, Key::RosterOrder, Key::Monitor] {
            let options = options(&gui, key);
            let mut sorted = options.clone();
            sorted.sort_by_key(|value| format!("{value}"));
            sorted.dedup();
            assert_eq!(
                sorted.len(),
                options.len(),
                "{} offers a duplicate option, which one step cannot leave",
                key.name()
            );
        }
    }

    /// `palettes` is a row of fixed choices with no key of its own, so it is
    /// the one row a reader might expect in `options`. It is deliberately not
    /// there: it writes three keys at once rather than selecting one of them,
    /// and it has no current value for a step to move away from. Pinned so the
    /// omission stays a decision instead of drifting into an oversight.
    #[test]
    fn the_palettes_row_is_not_an_option_select() {
        let palettes = FIELDS
            .iter()
            .find(|f| f.label == "palettes")
            .expect("the palettes row is registered");
        assert_eq!(
            palettes.key, None,
            "it edits no single key, so no step can name it"
        );
        assert_eq!(
            focus_target_of(palettes),
            None,
            "and it has no focus target yet: no keyless row is keyboard-reachable \
             before the per-field work, so nothing can step it"
        );
    }

    /// The focus ring is drawn by comparing `Gui::focus` against the target
    /// `focus_target_of` gives a row, and the tab order walks
    /// `rendered_targets`, so the rows that claim a config `Key` must be
    /// exactly the config keys: one row per key, no key claimed twice, no key
    /// missing. Compared as sets on purpose — the *order* is what the visual
    /// order means, and it is the next test's job to pin.
    #[test]
    fn every_config_field_declares_its_key_exactly_once() {
        let mut claimed: Vec<&str> = FIELDS.iter().filter_map(|f| f.key).map(Key::name).collect();
        claimed.sort_unstable();
        let mut expected: Vec<&str> = Key::ALL.iter().map(|k| k.name()).collect();
        expected.sort_unstable();
        assert_eq!(
            claimed, expected,
            "the rows claim a different set of config keys than the tab order walks"
        );
    }

    /// The two credential rows are the ones that must *not* claim a key, and
    /// the pinned set above is what makes that true: `Key::ALL` has no member
    /// for a client id, so a credential row given one would either duplicate a
    /// real key or fail that test outright. Asserted from the row side so a
    /// change to `Credential` alone cannot quietly introduce one.
    #[test]
    fn the_credential_rows_claim_no_config_key() {
        let credentials: Vec<&Field> = FIELDS
            .iter()
            .filter(|f| credential_of(f).is_some())
            .collect();
        assert_eq!(
            credentials.len(),
            CREDENTIAL_ROWS.len(),
            "every credential must name exactly one row, and no other row may claim one"
        );
        for field in credentials {
            assert_eq!(field.key, None, "{} claims a config key", field.label);
            assert_eq!(field.section, Section::Connection);
        }
    }

    /// `rendered_targets` is the tab order, and the tab order has to be the
    /// visual order: a mouse-only row drops out of it and the rest keep the
    /// sequence the page renders them in, which is `Section::ALL` order, not
    /// `FIELDS` order. One Layout row declared between two Position rows would
    /// tab fourth and paint ninth, so the grouping is the assertion.
    #[test]
    fn the_tab_order_follows_the_visual_order() {
        let order: Vec<&str> = rendered_targets("")
            .filter_map(|t| match t {
                FocusTarget::Field(key) => Some(key.name()),
                _ => None,
            })
            .collect();
        let rows: Vec<&str> = Section::ALL
            .iter()
            .flat_map(|section| FIELDS.iter().filter(move |f| f.section == *section))
            .filter_map(|f| f.key)
            .map(Key::name)
            .collect();
        assert_eq!(order, rows, "the tab order is not the row order");

        // `monitor` is the last Position row and `rtl` the third. Walking
        // `Key::ALL` instead puts them third and eighth, which scrolls the
        // viewport backwards and forwards on consecutive presses.
        assert_eq!(
            order.iter().position(|k| *k == "rtl"),
            Some(2),
            "rtl is the third row of the page, so it is the third field"
        );
        assert_eq!(
            order.iter().position(|k| *k == "monitor"),
            Some(7),
            "monitor is the eighth row of the page, so it is the eighth field"
        );
    }

    /// Adding the credential rows must not renumber the thirty keyed rows:
    /// they are rendered *after* every keyed row (Connection is the last
    /// section), so each keyed row keeps the Tab number it had when the
    /// credentials were mouse-only. Pinned as an exact sequence rather than a
    /// count, because a count would still pass if the rows swapped places.
    #[test]
    fn the_credential_rows_land_after_every_keyed_row() {
        let order: Vec<FocusTarget> = rendered_targets("").collect();
        let keyed: Vec<FocusTarget> = order
            .iter()
            .copied()
            .filter(|t| matches!(t, FocusTarget::Field(_)))
            .collect();
        assert_eq!(
            keyed.len(),
            Key::ALL.len(),
            "the keyed rows are still every config key, in visual order"
        );
        let first_credential = order
            .iter()
            .position(|t| matches!(t, FocusTarget::Credential(_)))
            .expect("the credential rows are in the tab order");
        assert_eq!(
            first_credential,
            keyed.len(),
            "a credential before the last keyed row would shift every Tab after it"
        );
        assert_eq!(
            order[first_credential..],
            [
                FocusTarget::Credential(Credential::ClientId),
                FocusTarget::Credential(Credential::ClientSecret),
            ],
            "the credentials tab in the order the page renders them"
        );
    }

    /// `view` picks the page on `gui.search.trim()` and `search_page` filters
    /// with the trimmed query, so `rendered_targets` has to trim too. It does
    /// not: a trailing space matches nothing at all and a lone space matches
    /// nothing, so either one leaves Tab with no row target at all.
    #[test]
    fn the_tab_order_reads_the_trimmed_query_the_page_does() {
        assert_eq!(
            rendered_targets("colors ").collect::<Vec<_>>(),
            [
                FocusTarget::Field(Key::SpeakingColor),
                FocusTarget::Field(Key::TextColor),
                FocusTarget::Field(Key::BoxColor),
            ],
            "a trailing space narrows nothing, so it must not narrow the tab order"
        );
        assert_eq!(
            rendered_targets(" ").count(),
            FIELDS.iter().filter_map(focus_target_of).count(),
            "a whitespace-only query is an empty one, and the page then renders \
             the one-pager"
        );
    }

    /// A search that names a credential row must reach it: the search page
    /// renders the same rows through the same `field_row`, so its focus target
    /// and its reveal id are the same two values.
    #[test]
    fn a_search_naming_a_credential_reaches_that_row() {
        let hits: Vec<FocusTarget> = rendered_targets("client secret").collect();
        assert!(
            hits.contains(&FocusTarget::Credential(Credential::ClientSecret)),
            "the secret row is a rendered hit, so Tab must be able to land on it"
        );
    }

    /// Every focusable row needs a reveal id, and a mouse-only row needs none:
    /// the id is what `scroll.rs` measures, so a missing one means Tab rings a
    /// row the page never scrolls into view.
    #[test]
    fn every_focusable_row_has_a_reveal_id_and_mouse_only_rows_have_none() {
        for field in FIELDS {
            let id = focus_target_of(field).and_then(row_id);
            if focus_target_of(field).is_some() {
                assert!(
                    id.is_some(),
                    "{} is focusable but carries no id",
                    field.label
                );
            } else {
                assert_eq!(
                    id, None,
                    "{} is mouse-only yet carries a reveal id",
                    field.label
                );
            }
        }
        // The two ids a credential needs are different, or the row container
        // and its input would collide in the widget tree.
        assert_ne!(
            Credential::ClientId.row_id(),
            Credential::ClientId.input_id()
        );
        assert_ne!(
            Credential::ClientId.row_id(),
            Credential::ClientSecret.row_id()
        );
        assert_ne!(
            Credential::ClientId.input_id(),
            Credential::ClientSecret.input_id()
        );
    }

    #[test]
    fn search_matches_label_tip_and_section_name() {
        let field = Field {
            section: Section::Position,
            label: "offset x",
            tip: "Horizontal distance in px from the anchored screen edge.",
            key: Some(Key::OffsetX),
            render: f_offset_x,
        };
        assert!(search_matches(&field, "offset"));
        assert!(search_matches(&field, "horizontal"));
        assert!(search_matches(&field, "POSITION"));
        assert!(!search_matches(&field, "avatar"));
        assert!(!search_matches(&field, ""));
    }

    #[test]
    fn sections_map_one_to_one_onto_config_groups() {
        use hyprlay_core::domain::Group;
        // Only config-backed sections participate; Connection has no group.
        let config_backed: Vec<_> = Section::ALL
            .into_iter()
            .filter_map(|section| section.group().map(|group| (section, group)))
            .collect();
        assert_eq!(config_backed.len(), Group::ALL.len());
        for ((section, group), expected) in config_backed.into_iter().zip(Group::ALL) {
            // The reset button sends one ResetGroup per GUI section; if a
            // section ever fails to map, its fields could never be reset.
            assert_eq!(group, expected);
            assert_eq!(
                format!("{expected}"),
                section.name().to_lowercase(),
                "section {} diverged from group {}",
                section.name(),
                group
            );
        }
    }

    /// Exactly one section is exempt from the reset machinery, and its name
    /// must stay stable because the shortcut hints and search rely on it.
    #[test]
    fn every_section_except_connection_maps_to_a_group() {
        for section in Section::ALL {
            match section.group() {
                Some(_) => assert_ne!(section.name(), "Connection"),
                None => assert_eq!(section.name(), "Connection"),
            }
        }
    }

    #[test]
    fn every_field_has_a_nonempty_tooltip() {
        for f in FIELDS {
            assert!(!f.tip.is_empty(), "field {} needs a tooltip", f.label);
            assert!(!f.label.is_empty());
        }
    }

    #[test]
    fn every_section_has_fields() {
        for s in Section::ALL {
            assert!(
                FIELDS.iter().any(|f| f.section == s),
                "section {} has no fields",
                s.name()
            );
        }
    }

    /// Click-through was removed; no field may render it again.
    #[test]
    fn click_through_is_gone_from_the_field_registry() {
        assert!(!FIELDS.iter().any(|f| f.label.contains("click")));
    }

    #[test]
    fn anchor_field_is_registered_in_the_position_section() {
        let field = FIELDS
            .iter()
            .find(|f| f.label == "anchor")
            .expect("anchor field registered");
        assert_eq!(field.section, Section::Position);
    }

    #[test]
    fn max_rows_field_is_registered_in_the_layout_section() {
        let field = FIELDS
            .iter()
            .find(|f| f.label == "max rows")
            .expect("max rows field registered");
        assert_eq!(field.section, Section::Layout);
    }

    #[test]
    fn roster_order_field_is_registered_in_the_layout_section() {
        let field = FIELDS
            .iter()
            .find(|f| f.label == "roster order")
            .expect("roster order field registered");
        assert_eq!(field.section, Section::Layout);
    }
}
