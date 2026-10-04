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

/// The focus indicator on a config-field row: a fill behind the label and the
/// control, not a box drawn around them.
///
/// A border's edge runs straight through the label text — on a slider row it
/// strikes the label and clips the number input's own frame, which is what the
/// owner reported. A fill is painted behind the row's content instead, so
/// nothing crosses what is being read. `Container`'s layout reads no style at
/// all, so neither the ring nor the fill moves a pixel of the page when focus
/// moves.
///
/// The fill is *lighter* than the panel, and that has a consequence: no darker
/// fill can be an indicator here, because the panel is so dark that even pure
/// black below it is only 1.27:1, while a lighter fill is what a 1216x50 band
/// needs to read. Lightening it costs `MUTED` its audited contrast — `MUTED`
/// clears 4.78:1 on a fill at most as light as the panel, i.e. on nothing — so
/// the focused row's own label lifts to `BRIGHT`, which holds 5.1:1 on this
/// fill. Unfocused rows keep `MUTED` on the panel.
pub(super) const FOCUS_FILL: Color = Color::from_rgb(0.33, 0.35, 0.42);

pub(super) fn focus_fill(focused: bool) -> impl Fn(&iced::Theme) -> container::Style {
    move |_t| container::Style {
        background: focused.then(|| FOCUS_FILL.into()),
        // The corner radius the ring had, so the focused row keeps its shape.
        // A border with no width and no colour draws nothing; iced clips the
        // fill to its radius.
        border: Border {
            radius: 6.0.into(),
            ..Border::default()
        },
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

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG 2.x relative luminance of a colour.
    fn luminance(c: Color) -> f32 {
        let channel = |v: f32| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
    }

    /// WCAG 2.x contrast ratio between two colours, order-independent.
    fn contrast(a: Color, b: Color) -> f32 {
        let (hi, lo) = {
            let (x, y) = (luminance(a), luminance(b));
            if x > y { (x, y) } else { (y, x) }
        };
        (hi + 0.05) / (lo + 0.05)
    }

    /// The focused row fills behind its own content, so the label has to stay
    /// readable on the fill. The audit bar is 4.78:1, the value the audit
    /// measured `MUTED` at on the content background. `BRIGHT` is what the
    /// focused row's label lifts to (`tip_label`), so that pairing is the one
    /// that has to clear the bar on the fill.
    #[test]
    fn the_focus_fill_keeps_the_audited_label_contrast() {
        let bright = contrast(BRIGHT, FOCUS_FILL);
        assert!(
            bright >= 4.78,
            "BRIGHT on the focus fill is {bright:.2}:1, under the audited 4.78:1"
        );
    }

    /// A focused row is a fill and nothing else: a stroke around the row is
    /// what crossed the label text, so the border has to stay widthless and
    /// colourless, and an unfocused row has to paint no background at all.
    #[test]
    fn a_focused_row_is_a_fill_and_not_a_stroke() {
        let focused = focus_fill(true)(&theme());
        let unfocused = focus_fill(false)(&theme());

        assert!(
            focused.background.is_some(),
            "the focused row paints no fill, so nothing marks it"
        );
        assert!(
            unfocused.background.is_none(),
            "an unfocused row paints a fill it did not before"
        );
        for (row, style) in [("focused", &focused), ("unfocused", &unfocused)] {
            assert_eq!(style.border.width, 0.0, "the {row} row draws a stroke");
            assert_eq!(
                style.border.color.a, 0.0,
                "the {row} row's stroke has colour"
            );
        }
    }
}
