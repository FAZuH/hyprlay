//! Color editor: swatch + hex input + reset, and an expandable HSV picker
//! (saturation/value square + hue strip + RGB sliders). Every interaction
//! reduces to a hex string fed into `Message::ColorHex`.

use hyprlay_core::color::Hsv;
use hyprlay_core::color::Rgb;
use hyprlay_core::config::Config;
use hyprlay_core::domain::Command;
use hyprlay_core::domain::HexColor;
use hyprlay_core::domain::Key;
use hyprlay_core::domain::Value;
use iced::Background;
use iced::Border;
use iced::Color;
use iced::Element;
use iced::Gradient;
use iced::Length;
use iced::Point;
use iced::Radians;
use iced::Task;
use iced::mouse;
use iced::widget::button;
use iced::widget::column;
use iced::widget::container;
use iced::widget::mouse_area;
use iced::widget::row;
use iced::widget::slider;
use iced::widget::stack;
use iced::widget::text;
use iced::widget::text_input;

use super::Gui;
use super::Message;
use super::fields::reset_button;
use super::update::update;

fn iced_color(rgb: Rgb) -> Color {
    Color::from_rgb(rgb.r, rgb.g, rgb.b)
}

// Color picker geometry.
const SV_W: f32 = 216.0;
const SV_H: f32 = 120.0;
const HUE_W: f32 = 216.0;
const HUE_H: f32 = 14.0;
const KNOB: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum ColorTarget {
    Speaking,
    Text,
    Box,
}

impl ColorTarget {
    /// The config key this target edits; keeps the GUI wired to the same
    /// vocabulary the wire protocol uses.
    pub(super) fn key(self) -> Key {
        match self {
            ColorTarget::Speaking => Key::SpeakingColor,
            ColorTarget::Text => Key::TextColor,
            ColorTarget::Box => Key::BoxColor,
        }
    }

    pub(super) fn field(self, cfg: &Config) -> HexColor {
        match self {
            ColorTarget::Speaking => cfg.speaking_color,
            ColorTarget::Text => cfg.text_color,
            ColorTarget::Box => cfg.box_color,
        }
    }

    pub(super) fn set_field(self, cfg: &mut Config, value: HexColor) {
        match self {
            ColorTarget::Speaking => cfg.speaking_color = value,
            ColorTarget::Text => cfg.text_color = value,
            ColorTarget::Box => cfg.box_color = value,
        }
    }

    pub(super) fn command(self, value: HexColor) -> Command {
        Command::Set(self.key(), Value::Color(value))
    }
}

/// One picker interaction: square point -> saturation/value -> hex command.
pub(super) fn apply_sv(gui: &mut Gui, target: ColorTarget, p: Point) -> Task<Message> {
    let hsv = current_hsv(gui, target);
    let s = (p.x / SV_W).clamp(0.0, 1.0);
    let v = 1.0 - (p.y / SV_H).clamp(0.0, 1.0);
    let hex = hyprlay_core::color::hex_from_hsv(Hsv { h: hsv.h, s, v });
    update(gui, Message::ColorHex(target, hex))
}

/// One picker interaction: strip point -> hue -> hex command.
pub(super) fn apply_hue(gui: &mut Gui, target: ColorTarget, p: Point) -> Task<Message> {
    let hsv = current_hsv(gui, target);
    let h = (p.x / HUE_W).clamp(0.0, 1.0) * 360.0;
    let hex = hyprlay_core::color::hex_from_hsv(Hsv {
        h,
        s: hsv.s,
        v: hsv.v,
    });
    update(gui, Message::ColorHex(target, hex))
}

fn current_hsv(gui: &Gui, target: ColorTarget) -> Hsv {
    hyprlay_core::color::hsv_from_rgb(Rgb::from(ColorTarget::field(target, &gui.config)))
}

/// Bridge between the app's validated colors and the framework-free math
/// in `hyprlay_core::color`; the only place the two type systems meet.
fn iced_hex(hex: HexColor) -> Color {
    let [r, g, b] = hex.rgb();
    Color::from_rgb8(r, g, b)
}

pub(super) fn color_editor(gui: &Gui, target: ColorTarget) -> Element<'_, Message> {
    let value = ColorTarget::field(target, &gui.config);
    let color = iced_hex(value);
    let defaults = Config::default();
    let default_hex = match target {
        ColorTarget::Speaking => defaults.speaking_color,
        ColorTarget::Text => defaults.text_color,
        ColorTarget::Box => defaults.box_color,
    };
    // While the typed text is invalid it lives in the draft buffer instead
    // of the config; a valid commit clears it.
    let hex = gui
        .drafts
        .get(&target)
        .cloned()
        .unwrap_or_else(|| value.to_string());

    let top = row![
        toggle_picker_button(target, color),
        text_input("#rrggbb", &hex)
            .width(Length::Fixed(110.0))
            .on_input(move |v| Message::ColorHex(target, v)),
        reset_button(Message::ColorHex(target, default_hex.to_string())),
    ]
    .spacing(8);

    if gui.picker == Some(target) {
        let hsv = hyprlay_core::color::hsv_from_rgb(Rgb {
            r: color.r,
            g: color.g,
            b: color.b,
        });
        column![
            top,
            sv_square(target, hsv),
            hue_strip(target, hsv),
            part_slider(target, 0, "R", color.r),
            part_slider(target, 1, "G", color.g),
            part_slider(target, 2, "B", color.b),
        ]
        .spacing(6)
        .into()
    } else {
        top.into()
    }
}

fn toggle_picker_button(target: ColorTarget, color: Color) -> Element<'static, Message> {
    button(
        container(
            iced::widget::Space::new()
                .width(Length::Fixed(26.0))
                .height(Length::Fixed(18.0)),
        )
        .style(move |_t| container::Style {
            background: Some(color.into()),
            border: Border {
                radius: 4.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        }),
    )
    .on_press(Message::PickerToggle(target))
    .style(|_t, _s| button::Style {
        border: Border {
            radius: 4.0.into(),
            ..Border::default()
        },
        ..button::Style::default()
    })
    .padding(0.0)
    .into()
}

fn sv_square(target: ColorTarget, hsv: Hsv) -> Element<'static, Message> {
    let knob_left = hsv.s * (SV_W - KNOB);
    let knob_top = (1.0 - hsv.v) * (SV_H - KNOB);
    let layers = stack![
        fill_layer(Background::Color(iced_color(hyprlay_core::color::hue_rgb(
            hsv.h
        )))),
        fill_layer(Background::Gradient(Gradient::Linear(
            iced::gradient::Linear::new(Radians(std::f32::consts::FRAC_PI_2))
                .add_stop(0.0, Color::WHITE)
                .add_stop(
                    1.0,
                    Color {
                        a: 0.0,
                        ..Color::WHITE
                    }
                ),
        ))),
        fill_layer(Background::Gradient(Gradient::Linear(
            iced::gradient::Linear::new(Radians(std::f32::consts::PI))
                .add_stop(
                    0.0,
                    Color {
                        a: 0.0,
                        ..Color::BLACK
                    }
                )
                .add_stop(1.0, Color::BLACK),
        ))),
        knob_layer(knob_left, knob_top),
    ];
    mouse_area(
        container(layers)
            .width(Length::Fixed(SV_W))
            .height(Length::Fixed(SV_H))
            .style(|_t| container::Style {
                border: Border {
                    radius: 4.0.into(),
                    color: Color::from_rgb(0.25, 0.26, 0.30),
                    width: 1.0,
                },
                ..container::Style::default()
            }),
    )
    .on_move(move |p| Message::SvMove(target, p))
    .on_press(Message::SvPress(target))
    .on_release(Message::PickerRelease)
    .interaction(mouse::Interaction::Crosshair)
    .into()
}

fn hue_strip(target: ColorTarget, hsv: Hsv) -> Element<'static, Message> {
    let knob_left = (hsv.h / 360.0) * (HUE_W - KNOB);
    let gradient = iced::gradient::Linear::new(Radians(std::f32::consts::FRAC_PI_2))
        .add_stop(0.0, Color::from_rgb8(255, 0, 0))
        .add_stop(1.0 / 6.0, Color::from_rgb8(255, 255, 0))
        .add_stop(2.0 / 6.0, Color::from_rgb8(0, 255, 0))
        .add_stop(3.0 / 6.0, Color::from_rgb8(0, 255, 255))
        .add_stop(4.0 / 6.0, Color::from_rgb8(0, 0, 255))
        .add_stop(5.0 / 6.0, Color::from_rgb8(255, 0, 255))
        .add_stop(1.0, Color::from_rgb8(255, 0, 0));
    let layers = stack![
        fill_layer(Background::Gradient(Gradient::Linear(gradient))),
        knob_layer(knob_left, (HUE_H - KNOB) / 2.0),
    ];
    mouse_area(
        container(layers)
            .width(Length::Fixed(HUE_W))
            .height(Length::Fixed(HUE_H))
            .style(|_t| container::Style {
                border: Border {
                    radius: 4.0.into(),
                    color: Color::from_rgb(0.25, 0.26, 0.30),
                    width: 1.0,
                },
                ..container::Style::default()
            }),
    )
    .on_move(move |p| Message::HueMove(target, p))
    .on_press(Message::HuePress(target))
    .on_release(Message::PickerRelease)
    .interaction(mouse::Interaction::Crosshair)
    .into()
}

fn fill_layer(background: Background) -> Element<'static, Message> {
    container(
        iced::widget::Space::new()
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .style(move |_t| container::Style {
        background: Some(background),
        ..container::Style::default()
    })
    .into()
}

/// A circular handle positioned by padding inside a filling layer.
fn knob_layer(left: f32, top: f32) -> Element<'static, Message> {
    let knob = container(
        iced::widget::Space::new()
            .width(Length::Fixed(KNOB))
            .height(Length::Fixed(KNOB)),
    )
    .style(|_t| container::Style {
        background: Some(Color::TRANSPARENT.into()),
        border: Border {
            color: Color::WHITE,
            width: 2.0,
            radius: (KNOB / 2.0).into(),
        },
        ..container::Style::default()
    });
    container(knob)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(iced::Padding {
            left,
            top,
            ..iced::Padding::default()
        })
        .into()
}

fn part_slider(
    target: ColorTarget,
    part: u8,
    label: &str,
    value: f32,
) -> Element<'static, Message> {
    row![
        text(label.to_string()).width(Length::Fixed(16.0)),
        slider(0.0..=1.0, value, move |v| Message::ColorPart(
            target, part, v
        ))
        .width(Length::Fill),
        text(format!("{}", (value * 255.0) as i64)).width(Length::Fixed(48.0)),
    ]
    .spacing(8)
    .into()
}

pub(super) fn swatch_dot(hex: HexColor) -> Element<'static, Message> {
    let color = iced_hex(hex);
    container(
        iced::widget::Space::new()
            .width(Length::Fixed(10.0))
            .height(Length::Fixed(10.0)),
    )
    .style(move |_t| container::Style {
        background: Some(color.into()),
        border: Border {
            radius: 3.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    })
    .into()
}
