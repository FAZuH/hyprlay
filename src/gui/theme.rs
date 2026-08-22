//! Settings-GUI visual identity: the fixed Discord-flavored dark palette,
//! the app theme, and reusable container/button/scrollbar styles.

use iced::Border;
use iced::Color;
use iced::Shadow;
use iced::widget::button;
use iced::widget::container;
use iced::widget::scrollable::AutoScroll;
use iced::widget::scrollable::Rail;
use iced::widget::scrollable::Scroller;
use iced::widget::scrollable::{self};

// Panel shades: header darkest, sidebar slightly lifted, content on theme bg.
pub(super) const HEADER_BG: Color = Color::from_rgb(0.090, 0.094, 0.106); // #17181b
pub(super) const SIDEBAR_BG: Color = Color::from_rgb(0.103, 0.106, 0.118); // #1a1b1e
pub(super) const FIELD_BG: Color = Color::from_rgb(0.160, 0.170, 0.200);
pub(super) const MUTED: Color = Color::from_rgb(0.50, 0.51, 0.55);
pub(super) const BRIGHT: Color = Color::from_rgb(0.86, 0.87, 0.88);
pub(super) const ACCENT: Color = Color::from_rgb(0.345, 0.396, 0.949);
pub(super) const ACCENT_LIT: Color = Color::from_rgb(0.42, 0.48, 0.98);
pub(super) const AMBER: Color = Color::from_rgb(0.96, 0.72, 0.24);
pub(super) const REPLY_GREEN: Color = Color::from_rgb(0.42, 0.72, 0.47);

pub(super) fn theme_for(_gui: &super::Gui) -> iced::Theme {
    theme()
}

fn theme() -> iced::Theme {
    iced::Theme::custom(
        "hyprlay",
        iced::theme::Palette {
            background: Color::from_rgb(0.118, 0.121, 0.133), // #1e1f22
            text: BRIGHT,
            primary: ACCENT,
            success: Color::from_rgb(0.13, 0.77, 0.37),
            warning: AMBER,
            danger: Color::from_rgb(0.95, 0.25, 0.26),
        },
    )
}

pub(super) fn scrollbar_style(
    _theme: &iced::Theme,
    _status: scrollable::Status,
) -> scrollable::Style {
    let rail = Rail {
        background: Some(Color::from_rgba(0.09, 0.09, 0.11, 0.6).into()),
        border: Border::default(),
        scroller: Scroller {
            background: Color::from_rgb(0.42, 0.44, 0.50).into(),
            border: Border::default(),
        },
    };
    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
        auto_scroll: AutoScroll {
            background: Color::TRANSPARENT.into(),
            border: Border::default(),
            shadow: Shadow::default(),
            icon: Color::TRANSPARENT,
        },
    }
}

pub(super) fn panel(bg: Color) -> impl Fn(&iced::Theme) -> container::Style {
    move |_t| container::Style {
        background: Some(bg.into()),
        ..container::Style::default()
    }
}

pub(super) fn nav_style(selected: bool) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_t, _s| button::Style {
        background: Some(if selected { ACCENT_LIT } else { FIELD_BG }.into()),
        text_color: Color::WHITE,
        border: Border {
            radius: 6.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}

pub(super) fn plain_style() -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_t, s| {
        // Disabled buttons (e.g. "Clear changes" on a clean config) darken
        // below even the panel background and dim their label so the press
        // target visibly reads as inert next to its enabled neighbors.
        let (background, text_color) = match s {
            button::Status::Disabled => (
                Color::from_rgb(0.108, 0.112, 0.130),
                Color::from_rgb(0.36, 0.37, 0.40),
            ),
            _ => (FIELD_BG, BRIGHT),
        };
        button::Style {
            background: Some(background.into()),
            text_color,
            border: Border {
                radius: 6.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        }
    }
}

pub(super) fn primary_style(
    active: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_t: &iced::Theme, s: button::Status| button::Style {
        background: Some(
            if matches!(s, button::Status::Hovered) || active {
                ACCENT_LIT
            } else {
                ACCENT
            }
            .into(),
        ),
        text_color: Color::WHITE,
        border: Border {
            radius: 6.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    }
}
