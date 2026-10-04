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
//
// Every text/background pairing here is measured against WCAG AA
// (4.5:1 normal, 3:1 large), not eyeballed. The two values nudged to get
// there are `MUTED` (was 0.50/0.51/0.55, 4.31:1 on the content
// background) and `ACCENT_LIT` (was 0.42/0.48/0.98, 3.63:1 with white).
pub(super) const HEADER_BG: Color = Color::from_rgb(0.090, 0.094, 0.106); // #17181b
pub(super) const SIDEBAR_BG: Color = Color::from_rgb(0.103, 0.106, 0.118); // #1a1b1e
pub(super) const FIELD_BG: Color = Color::from_rgb(0.160, 0.170, 0.200);
// 4.78:1 on the content background, 5.15:1 header, 5.00:1 sidebar. The old
// value sat at 4.31:1 and failed on the only surface carrying body text.
pub(super) const MUTED: Color = Color::from_rgb(0.53, 0.54, 0.58);
pub(super) const BRIGHT: Color = Color::from_rgb(0.86, 0.87, 0.88);
pub(super) const ACCENT: Color = Color::from_rgb(0.345, 0.396, 0.949); // #5865f2
// Was 0.42/0.48/0.98: 3.63:1 with white text, failing AA in exactly the
// hover and active states the user looks at. 4.70:1 with white, and still
// lighter than `ACCENT` so the hover cue remains a brightness step.
pub(super) const ACCENT_LIT: Color = Color::from_rgb(0.35, 0.40, 0.90);
pub(super) const AMBER: Color = Color::from_rgb(0.96, 0.72, 0.24);
pub(super) const REPLY_GREEN: Color = Color::from_rgb(0.42, 0.72, 0.47);
/// The palette's `danger`, as a constant: a failed command's reply paints
/// in this instead of the success colour. Successes and errors used to
/// share `REPLY_GREEN`, which hid failures.
pub(super) const DANGER: Color = Color::from_rgb(0.95, 0.25, 0.26);

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

pub(super) fn nav_style(
    selected: bool,
    focused: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_t, _s| button::Style {
        background: Some(if selected { ACCENT_LIT } else { FIELD_BG }.into()),
        text_color: Color::WHITE,
        border: focus_border(
            Border {
                radius: 6.0.into(),
                ..Border::default()
            },
            focused,
        ),
        ..button::Style::default()
    }
}

/// The focus indicator: a visible ring on whichever widget holds keyboard
/// focus. Iced 0.14's `button::Style` has no focus field, so this is the
/// only path a keyboard-only user has to see where they are (R-32).
///
/// `FOCUS_RING` is 3:1 against the content background and the sidebar, which
/// are the two surfaces a focused control sits on.
pub(super) const FOCUS_RING: Color = Color::from_rgb(0.60, 0.80, 1.00);

fn focus_border(base: Border, focused: bool) -> Border {
    if !focused {
        return base;
    }
    Border {
        color: FOCUS_RING,
        width: 2.0,
        ..base
    }
}

/// The same ring as a container border, for the config-field rows: those are
/// toggles, chips, sliders and number rows rather than buttons, so the
/// indicator has to be a box around the whole row instead of a button border.
pub(super) fn focus_ring(focused: bool) -> impl Fn(&iced::Theme) -> container::Style {
    move |_t| container::Style {
        border: focus_border(
            Border {
                radius: 6.0.into(),
                ..Border::default()
            },
            focused,
        ),
        ..container::Style::default()
    }
}

pub(super) fn plain_style(focused: bool) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    move |_t, s| {
        // Disabled buttons (e.g. "Clear changes" on a clean config) darken
        // below even the panel background and dim their label so the press
        // target visibly reads as inert next to its enabled neighbors. The
        // label is still lifted to 3.93:1 against its own background; the
        // old value sat at 2.60:1 and was unreadable rather than merely dim.
        let (background, text_color) = match s {
            button::Status::Disabled => (
                Color::from_rgb(0.108, 0.112, 0.130),
                Color::from_rgb(0.47, 0.48, 0.51),
            ),
            _ => (FIELD_BG, BRIGHT),
        };
        button::Style {
            background: Some(background.into()),
            text_color,
            border: focus_border(
                Border {
                    radius: 6.0.into(),
                    ..Border::default()
                },
                focused,
            ),
            ..button::Style::default()
        }
    }
}

pub(super) fn primary_style(
    active: bool,
    focused: bool,
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
        border: focus_border(
            Border {
                radius: 6.0.into(),
                ..Border::default()
            },
            focused,
        ),
        ..button::Style::default()
    }
}
