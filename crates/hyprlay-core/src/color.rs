//! Linear RGB / HSV color math. Pure arithmetic over plain float channels —
//! no rendering-framework types — so any UI can convert at its own edge.

/// Additive RGB, each channel 0.0..=1.0.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rgb {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Rgb {
    pub const fn from_rgb8(r: u8, g: u8, b: u8) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
        }
    }
}

/// Hue 0..=360, saturation and value 0..=1.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Hsv {
    pub h: f32,
    pub s: f32,
    pub v: f32,
}

pub fn hsv_from_rgb(rgb: Rgb) -> Hsv {
    let (r, g, b) = (rgb.r, rgb.g, rgb.b);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;

    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / delta) % 6.0)
    } else if max == g {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };
    let h = if h < 0.0 { h + 360.0 } else { h };

    Hsv {
        h,
        s: if max == 0.0 { 0.0 } else { delta / max },
        v: max,
    }
}

pub fn rgb_from_hsv(hsv: Hsv) -> Rgb {
    let Hsv { h, s, v } = hsv;
    let c = v * s;
    let hp = (h % 360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    Rgb {
        r: r1 + m,
        g: g1 + m,
        b: b1 + m,
    }
}

/// `#rrggbb` for an HSV triple, rounded to the nearest byte per channel.
#[must_use]
pub fn hex_from_hsv(hsv: Hsv) -> String {
    let rgb = rgb_from_hsv(hsv);
    let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(rgb.r), byte(rgb.g), byte(rgb.b))
}

/// The pure hue at full saturation/value — the base fill of a picker's
/// saturation/value square.
pub fn hue_rgb(h: f32) -> Rgb {
    rgb_from_hsv(Hsv { h, s: 1.0, v: 1.0 })
}

impl From<crate::domain::HexColor> for Rgb {
    fn from(hex: crate::domain::HexColor) -> Self {
        let [r, g, b] = hex.rgb();
        Self::from_rgb8(r, g, b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb::from_rgb8(r, g, b)
    }

    #[test]
    fn hsv_roundtrips_primary_and_secondary_colors() {
        for &c in &[
            rgb(0xff, 0x00, 0x00),
            rgb(0x00, 0xff, 0x00),
            rgb(0x00, 0x00, 0xff),
            rgb(0xff, 0xff, 0x00),
            rgb(0x00, 0xff, 0xff),
            rgb(0xff, 0x00, 0xff),
        ] {
            let back = rgb_from_hsv(hsv_from_rgb(c));
            // Compare as bytes: the roundtrip is exact up to 8-bit rounding.
            let byte = |x: f32| (x * 255.0).round() as i16;
            assert_eq!(
                (byte(back.r), byte(back.g), byte(back.b)),
                (byte(c.r), byte(c.g), byte(c.b))
            );
        }
    }

    #[test]
    fn hsv_of_red_is_zero_hue() {
        let hsv = hsv_from_rgb(rgb(255, 0, 0));
        assert_eq!((hsv.h, hsv.s, hsv.v), (0.0, 1.0, 1.0));
    }

    #[test]
    fn hsv_of_black_has_no_hue_and_no_saturation() {
        let hsv = hsv_from_rgb(rgb(0, 0, 0));
        assert_eq!((hsv.h, hsv.s, hsv.v), (0.0, 0.0, 0.0));
    }

    #[test]
    fn hsv_of_gray_keeps_value_only() {
        let hsv = hsv_from_rgb(rgb(255, 255, 255));
        assert_eq!((hsv.s, hsv.v), (0.0, 1.0));
    }

    #[test]
    fn hue_of_green_is_120() {
        let hsv = hsv_from_rgb(rgb(0, 255, 0));
        assert!((hsv.h - 120.0).abs() < 0.001, "h was {}", hsv.h);
    }

    #[test]
    fn hex_from_hsv_matches_known_colors() {
        let hsv = |h, s, v| Hsv { h, s, v };
        assert_eq!(hex_from_hsv(hsv(0.0, 1.0, 1.0)), "#ff0000");
        assert_eq!(hex_from_hsv(hsv(120.0, 1.0, 1.0)), "#00ff00");
        assert_eq!(hex_from_hsv(hsv(240.0, 1.0, 1.0)), "#0000ff");
        assert_eq!(hex_from_hsv(hsv(0.0, 0.0, 0.0)), "#000000");
        assert_eq!(hex_from_hsv(hsv(0.0, 0.0, 1.0)), "#ffffff");
    }

    #[test]
    fn rgb_from_hsv_clamps_hue_above_360() {
        // 390 degrees is the same color as 30 degrees (orange).
        let a = rgb_from_hsv(Hsv {
            h: 390.0,
            s: 1.0,
            v: 1.0,
        });
        let b = rgb_from_hsv(Hsv {
            h: 30.0,
            s: 1.0,
            v: 1.0,
        });
        assert_eq!((a.r, a.g, a.b), (b.r, b.g, b.b));
    }

    #[test]
    fn hue_rgb_at_zero_is_red() {
        assert_eq!(hue_rgb(0.0), rgb(255, 0, 0));
    }
}
