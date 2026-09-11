//! The mute/deafen mark: which glyph a roster row shows and in which color.
//!
//! The decision is kept here, free of widgets, so the pins below hold no
//! matter how the row is drawn.

use iced::Color;

use crate::daemon::adapters::discord::Participant;

/// Icon path data from the Material Design Icons set (Pictogrammers Free
/// License, Apache-2.0): the crossed-out microphone and crossed-out
/// headphones. Copied as path data only, in a 24×24 viewBox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Glyph {
    MicOff,
    HeadphonesOff,
}

const MIC_OFF: &str = r#"<svg viewBox="0 0 24 24"><path d="M19,11C19,12.19 18.66,13.3 18.1,14.28L16.87,13.05C17.14,12.43 17.3,11.74 17.3,11H19M15,11.16L9,5.18V5A3,3 0 0,1 12,2A3,3 0 0,1 15,5V11L15,11.16M4.27,3L21,19.73L19.73,21L15.54,16.81C14.77,17.27 13.91,17.58 13,17.72V21H11V17.72C7.72,17.23 5,14.41 5,11H6.7C6.7,14 9.24,16.1 12,16.1C12.81,16.1 13.6,15.91 14.31,15.58L12.65,13.92L12,14A3,3 0 0,1 9,11V10.28L3,4.27L4.27,3Z"/></svg>"#;

const HEADPHONES_OFF: &str = r#"<svg viewBox="0 0 24 24"><path d="M12,1A9,9 0 0,1 21,10V17C21,17.62 20.81,18.19 20.5,18.67L15,13.18V12H19V10A7,7 0 0,0 12,3C10,3 8.23,3.82 6.96,5.14L5.55,3.72C7.18,2.04 9.47,1 12,1M2.78,3.5L20.5,21.22L19.23,22.5L16.73,20H15V18.27L9,12.27V20H6A3,3 0 0,1 3,17V10C3,8.89 3.2,7.82 3.57,6.84L1.5,4.77L2.78,3.5M5.17,8.44C5.06,8.94 5,9.46 5,10V12H8.73L5.17,8.44Z"/></svg>"#;

impl Glyph {
    pub(crate) fn svg(self) -> &'static str {
        match self {
            Self::MicOff => MIC_OFF,
            Self::HeadphonesOff => HEADPHONES_OFF,
        }
    }
}

/// Tailwind `red-600`: a moderator silenced this participant.
const SERVER_COLOR: Color = Color::from_rgb8(0xDC, 0x26, 0x26);
/// Tailwind `neutral-400`: they silenced themselves.
const SELF_COLOR: Color = Color::from_rgb8(0xA3, 0xA3, 0xA3);

/// The one badge a row carries, if it carries one at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Mark {
    pub glyph: Glyph,
    pub server_caused: bool,
}

impl Mark {
    pub(crate) fn color(self) -> Color {
        if self.server_caused {
            SERVER_COLOR
        } else {
            SELF_COLOR
        }
    }
}

/// The mark a participant earns, or none when they are clean. Deafening
/// wins over muting: one badge per row, never two. Server-set flags decide
/// the color on their own, so a self-deafened participant who is also
/// server-muted still reads red.
pub(crate) fn mark_of(p: &Participant) -> Option<Mark> {
    let glyph = if p.deafened() {
        Glyph::HeadphonesOff
    } else if p.muted() {
        Glyph::MicOff
    } else {
        return None;
    };
    Some(Mark {
        glyph,
        server_caused: p.server_mute || p.server_deaf,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(
        self_mute: bool,
        self_deaf: bool,
        server_mute: bool,
        server_deaf: bool,
    ) -> Participant {
        Participant {
            id: "42".into(),
            name: "fazuh".into(),
            avatar_hash: None,
            speaking: false,
            self_mute,
            self_deaf,
            server_mute,
            server_deaf,
        }
    }

    /// `(self_mute, self_deaf, server_mute, server_deaf)` and the mark (glyph
    /// + server-caused) it must produce, if any.
    type Case = ((bool, bool, bool, bool), Option<(Glyph, bool)>);

    const CASES: [Case; 16] = [
        ((false, false, false, false), None),
        ((true, false, false, false), Some((Glyph::MicOff, false))),
        (
            (false, true, false, false),
            Some((Glyph::HeadphonesOff, false)),
        ),
        (
            (true, true, false, false),
            Some((Glyph::HeadphonesOff, false)),
        ),
        ((false, false, true, false), Some((Glyph::MicOff, true))),
        ((true, false, true, false), Some((Glyph::MicOff, true))),
        (
            (false, true, true, false),
            Some((Glyph::HeadphonesOff, true)),
        ),
        (
            (true, true, true, false),
            Some((Glyph::HeadphonesOff, true)),
        ),
        (
            (false, false, false, true),
            Some((Glyph::HeadphonesOff, true)),
        ),
        (
            (true, false, false, true),
            Some((Glyph::HeadphonesOff, true)),
        ),
        (
            (false, true, false, true),
            Some((Glyph::HeadphonesOff, true)),
        ),
        (
            (true, true, false, true),
            Some((Glyph::HeadphonesOff, true)),
        ),
        (
            (false, false, true, true),
            Some((Glyph::HeadphonesOff, true)),
        ),
        (
            (true, false, true, true),
            Some((Glyph::HeadphonesOff, true)),
        ),
        (
            (false, true, true, true),
            Some((Glyph::HeadphonesOff, true)),
        ),
        ((true, true, true, true), Some((Glyph::HeadphonesOff, true))),
    ];

    #[test]
    fn mark_of_maps_every_mute_flag_combination_to_one_glyph() {
        for ((sm, sd, vm, vd), expected) in CASES {
            let got = mark_of(&state(sm, sd, vm, vd)).map(|m| (m.glyph, m.server_caused));
            assert_eq!(
                got, expected,
                "self_mute={sm} self_deaf={sd} server_mute={vm} server_deaf={vd}"
            );
        }
    }

    #[test]
    fn clean_participant_gets_no_mark() {
        assert_eq!(mark_of(&state(false, false, false, false)), None);
    }

    #[test]
    fn server_color_is_red_600_and_self_color_is_neutral_400() {
        let red = Mark {
            glyph: Glyph::MicOff,
            server_caused: true,
        }
        .color();
        let grey = Mark {
            glyph: Glyph::MicOff,
            server_caused: false,
        }
        .color();
        assert_eq!(red, Color::from_rgb8(0xDC, 0x26, 0x26));
        assert_eq!(grey, Color::from_rgb8(0xA3, 0xA3, 0xA3));
    }

    #[test]
    fn both_glyphs_are_24by24_viewbox_paths() {
        for glyph in [Glyph::MicOff, Glyph::HeadphonesOff] {
            let doc = glyph.svg();
            assert!(doc.starts_with(r#"<svg viewBox="0 0 24 24">"#), "{doc}");
            assert!(doc.contains("<path d=\"M"), "{doc}");
        }
    }
}
