//! Overlay view: one row per displayed participant floating on a fully
//! transparent surface. All sizes and colors come from the config.

use hyprlay_core::config::Alphas;
use hyprlay_core::domain::HexColor;
use iced::Border;
use iced::Color;
use iced::Element;
use iced::Length;
use iced::widget::Space;
use iced::widget::column;
use iced::widget::container;
use iced::widget::container::Style as ContainerStyle;
use iced::widget::image;
use iced::widget::row;
use iced::widget::text;

use crate::daemon::Message;
use crate::daemon::adapters::discord::Participant;
use crate::daemon::overlay::state::Overlay;

/// Bridge from the core-validated `#rrggbb` type to the renderer's color.
/// A free function rather than a `From` impl: both types are foreign here,
/// so the orphan rule forbids the impl.
fn color_of(hex: HexColor) -> Color {
    let [r, g, b] = hex.rgb();
    Color::from_rgb8(r, g, b)
}

const MUTE_RED: Color = Color::from_rgb(0.95, 0.25, 0.26);
const DEAF_ORANGE: Color = Color::from_rgb(0.96, 0.72, 0.24);

const FALLBACK_COLORS: [Color; 6] = [
    Color::from_rgb(0.36, 0.44, 0.96),
    Color::from_rgb(0.85, 0.32, 0.42),
    Color::from_rgb(0.24, 0.66, 0.53),
    Color::from_rgb(0.87, 0.60, 0.22),
    Color::from_rgb(0.52, 0.40, 0.87),
    Color::from_rgb(0.25, 0.60, 0.85),
];

fn scaled(state: &Overlay, v: u32) -> f32 {
    v as f32 * state.config().scale_f32()
}

pub fn view<'a>(state: &'a Overlay) -> Element<'a, Message> {
    // Four knobs: overall opacity multiplies into the profile picture, the
    // username text, and the username background. At 100/100/100/100 nothing
    // is transparent anywhere.
    let alphas = state.effective_alphas();

    let rows: Vec<Element<Message>> = state
        .displayed()
        .into_iter()
        .map(|p| participant_row(state, p, alphas))
        .collect();

    // Fully transparent panel: no background, no border — only the rows.
    // Connect/sign-in progress is deliberately never rendered: an empty
    // roster is an empty surface, so the overlay only ever shows people.
    column(rows)
        .spacing(scaled(state, state.config().spacing))
        .into()
}

fn participant_row<'a>(
    state: &'a Overlay,
    p: &'a Participant,
    alphas: Alphas,
) -> Element<'a, Message> {
    let avatar_px = scaled(state, state.config().avatar_size);
    let speaking = p.speaking;

    let avatar: Element<'_, Message> = match state.avatar(&p.id) {
        Some(handle) => image::Image::new(handle.clone())
            .width(Length::Fixed(avatar_px))
            .height(Length::Fixed(avatar_px))
            .border_radius(iced::border::Radius::from(avatar_px / 2.0))
            .opacity(alphas.avatar)
            .into(),
        None => fallback_avatar(&p.id, &p.name, avatar_px, alphas.avatar),
    };

    // Speaking ring hugs the circular avatar. Constant padding + border
    // width whether speaking or not, so toggling the ring never shifts the
    // avatar's position (only its color changes). Owned values so the style
    // closure captures no reference to `state`.
    let ring = Border {
        color: if speaking {
            Color {
                a: alphas.overall,
                ..color_of(state.config().speaking_color)
            }
        } else {
            Color::TRANSPARENT
        },
        width: 2.0,
        radius: iced::border::Radius::from((avatar_px + 8.0) / 2.0),
    };
    let avatar: Element<'_, Message> = container(avatar)
        .padding(2.0)
        .style(move |_t| ContainerStyle {
            border: ring,
            ..ContainerStyle::default()
        })
        .into();

    let name = truncate(&p.name, state.config().max_username_length);
    // Speakers are differentiated by the ring only — names stay fully
    // opaque so the per-part sliders are the only transparency knobs.
    let name_color = Color {
        a: alphas.text,
        ..color_of(state.config().text_color)
    };
    let name_el = text(name)
        .size(scaled(state, state.config().text_size))
        .color(name_color);

    // The background chip always renders; box-opacity 0 makes it invisible.
    let chip_bg = Color {
        a: alphas.box_bg,
        ..color_of(state.config().box_color)
    };
    let chip_radius = iced::border::Radius::from(scaled(state, state.config().text_size) * 0.6);
    let name_area: Element<'_, Message> = container(name_el)
        .padding([2, 8])
        .style(move |_t| ContainerStyle {
            background: Some(chip_bg.into()),
            border: Border {
                radius: chip_radius,
                ..Border::default()
            },
            ..ContainerStyle::default()
        })
        .into();

    let mut badges = row![].spacing(4.0);
    if p.muted() {
        badges = badges.push(badge("M", MUTE_RED, alphas.text, alphas.box_bg, state));
    }
    if p.deafened() {
        badges = badges.push(badge("D", DEAF_ORANGE, alphas.text, alphas.box_bg, state));
    }

    if state.config().rtl {
        // Avatar on the right, name to its left, text right-aligned.
        row![
            Space::new().width(Length::Fill),
            name_right_aligned(state, name_area),
            badges,
            avatar
        ]
        .spacing(8.0)
        .align_y(iced::Alignment::Center)
        .into()
    } else {
        row![avatar, name_area, Space::new().width(Length::Fill), badges]
            .spacing(8.0)
            .align_y(iced::Alignment::Center)
            .into()
    }
}

/// In RTL mode the name hugs the avatar: right-aligned inside a filling row.
fn name_right_aligned<'a>(
    state: &Overlay,
    name_area: Element<'a, Message>,
) -> Element<'a, Message> {
    let text_size = scaled(state, state.config().text_size);
    container(name_area)
        .width(Length::Fill)
        .align_x(iced::alignment::Horizontal::Right)
        .padding(iced::Padding {
            left: text_size,
            ..Default::default()
        })
        .into()
}

fn badge(
    label: &str,
    color: Color,
    alpha: f32,
    bg_alpha: f32,
    state: &Overlay,
) -> Element<'static, Message> {
    let size = scaled(state, state.config().text_size) * 0.7;
    let bg = Color {
        a: bg_alpha,
        ..color
    };
    container(text(label.to_string()).size(size).color(Color {
        a: alpha,
        ..Color::WHITE
    }))
    .padding(2.0)
    .style(move |_t| ContainerStyle {
        background: Some(bg.into()),
        border: Border {
            radius: (size * 0.5).into(),
            ..Border::default()
        },
        ..ContainerStyle::default()
    })
    .into()
}

fn fallback_avatar(id: &str, name: &str, px: f32, alpha: f32) -> Element<'static, Message> {
    let color = FALLBACK_COLORS[id.bytes().map(usize::from).sum::<usize>() % FALLBACK_COLORS.len()];
    let bg = Color { a: alpha, ..color };
    let initial = name
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();
    container(text(initial).size(px * 0.45).color(Color {
        a: alpha,
        ..Color::WHITE
    }))
    .width(Length::Fixed(px))
    .height(Length::Fixed(px))
    .align_x(iced::alignment::Horizontal::Center)
    .align_y(iced::alignment::Vertical::Center)
    .style(move |_t| ContainerStyle {
        background: Some(bg.into()),
        border: Border {
            radius: (px / 2.0).into(),
            ..Border::default()
        },
        ..ContainerStyle::default()
    })
    .into()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_returns_short_names_unchanged() {
        assert_eq!(truncate("fazuh", 16), "fazuh");
    }

    #[test]
    fn truncate_keeps_exactly_at_limit_unchanged() {
        assert_eq!(truncate("0123456789abcdef", 16), "0123456789abcdef");
    }

    #[test]
    fn truncate_cuts_over_limit_and_appends_ellipsis() {
        assert_eq!(truncate("0123456789abcdefg", 16), "0123456789abcde…");
    }

    #[test]
    fn truncate_counts_chars_not_bytes() {
        // 4 unicode chars fit in a limit of 4 despite being 16 bytes.
        assert_eq!(truncate("日本語だ", 4), "日本語だ");
    }
}
