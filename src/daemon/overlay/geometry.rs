//! Surface placement: where the layer-shell surface sits on screen and how
//! drag deltas move it. Pure functions over (Config, margins) — no runtime
//! state, no iced types beyond the Wayland anchor vocabulary.

use hyprlay_core::compositor::Monitor;
use hyprlay_core::config::AnchorMode;
use hyprlay_core::config::Config;
use hyprlay_core::config::HorizontalAnchor;
use hyprlay_core::config::VerticalAnchor;

/// Margins are clamped so a long drag session can't overflow i32 or push
/// the surface into undefined compositor territory.
const MARGIN_LIMIT: i32 = 8000;

/// The vertical edge the surface actually glues to: `Auto` defers to the
/// position's vertical side; an explicit anchor overrides it. Drag/nudge
/// accumulation must branch on this resolved edge — nudging while
/// bottom-glued over a top position otherwise accumulates on the top margin
/// while the surface hangs from the bottom, so offsets fight the anchor.
fn effective_vertical(cfg: &Config) -> VerticalAnchor {
    match cfg.anchor {
        AnchorMode::Auto => cfg.vertical,
        AnchorMode::Top => VerticalAnchor::Top,
        AnchorMode::Bottom => VerticalAnchor::Bottom,
    }
}

/// Wayland anchors for the configured screen edge: top-left config anchors
/// top+left, center adds both horizontal anchors (fixed size → centered).
/// The `anchor` config overrides the vertical glue edge — anchored top the
/// list grows downward as users join, anchored bottom it grows upward, so
/// the overlay never runs off screen.
///
/// Linux/Wayland-only: this returns the layer-shell `Anchor` type, which does
/// not exist on the winit arm. The winit arm positions the window absolutely
/// via [`winit_frame`] instead.
#[cfg(target_os = "linux")]
pub fn anchor(cfg: &Config) -> iced_layershell::reexport::Anchor {
    use iced_layershell::reexport::Anchor;
    let horizontal = match cfg.horizontal {
        HorizontalAnchor::Left => Anchor::Left,
        HorizontalAnchor::Right => Anchor::Right,
        HorizontalAnchor::Center => Anchor::Left | Anchor::Right,
    };
    let vertical = match effective_vertical(cfg) {
        VerticalAnchor::Top => Anchor::Top,
        VerticalAnchor::Bottom => Anchor::Bottom,
    };
    horizontal | vertical
}

/// Initial (top, right, bottom, left) layer-shell margin for the configured
/// anchor — the user-facing `offset_x`/`offset_y` pushed onto anchored edges.
pub fn offset(cfg: &Config) -> (i32, i32, i32, i32) {
    (cfg.offset_y, cfg.offset_x, cfg.offset_y, cfg.offset_x)
}

pub fn clamp(v: i32) -> i32 {
    v.clamp(-MARGIN_LIMIT, MARGIN_LIMIT)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub fn contains(&self, point: (i32, i32)) -> bool {
        let (px, py) = point;
        px >= self.x && px < self.x + self.w as i32 && py >= self.y && py < self.y + self.h as i32
    }
}

pub fn overlay_rect(
    cfg: &Config,
    size: (u32, u32),
    offset: (i32, i32, i32, i32),
    monitor: Option<&hyprlay_core::compositor::Monitor>,
) -> Rect {
    let (top, right, bottom, left) = offset;
    let (w, h) = size;
    let (mx, my, mw, mh) = monitor.map(monitor_logical).unwrap_or((0, 0, 0, 0));
    let x = match cfg.horizontal {
        HorizontalAnchor::Left => mx + left,
        HorizontalAnchor::Right => {
            if mw == 0 {
                mx + left
            } else {
                mx + mw - w as i32 - right
            }
        }
        HorizontalAnchor::Center => {
            if mw == 0 {
                mx + left
            } else {
                mx + (mw - w as i32) / 2 + (left - right) / 2
            }
        }
    };
    let y = match effective_vertical(cfg) {
        VerticalAnchor::Top => my + top,
        VerticalAnchor::Bottom => {
            if mh == 0 {
                my + top
            } else {
                my + mh - h as i32 - bottom
            }
        }
    };
    Rect { x, y, w, h }
}

/// The monitor the overlay should sit on: the named one if configured and
/// present, else the currently-active output, else none. Pure so the winit
/// arm can resolve a target from either the [`Compositor`] port or an
/// iced/winit monitor list with the same rules (and the same unit tests).
pub fn pick_monitor<'a>(monitors: &'a [Monitor], name: Option<&str>) -> Option<&'a Monitor> {
    if let Some(name) = name
        && let Some(m) = monitors.iter().find(|m| m.name == name)
    {
        return Some(m);
    }
    monitors.iter().find(|m| m.active)
}

/// One monitor's logical (scale-corrected) extent in global screen coords:
/// its origin plus its width/height divided by the scale factor. Layer-shell
/// and winit both reason about logical pixels, so a scaled output's physical
/// geometry is normalised here before any placement math.
pub fn monitor_logical(m: &Monitor) -> (i32, i32, i32, i32) {
    let scale = if m.scale == 0.0 { 1.0 } else { m.scale };
    let w = (m.width as f32 / scale) as i32;
    let h = (m.height as f32 / scale) as i32;
    (m.x, m.y, w, h)
}

/// The winit arm's on-screen frame: the top-left corner and size of the
/// overlay window in logical pixels, for a given monitor. Reuses the same
/// anchor/margin placement math as [`overlay_rect`] — the two arms agree on
/// where the overlay sits, only how it is *applied* to the surface differs.
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub fn winit_frame(
    cfg: &Config,
    size: (u32, u32),
    offset: (i32, i32, i32, i32),
    monitor: Option<&Monitor>,
) -> WinitFrame {
    let rect = overlay_rect(cfg, size, offset, monitor);
    WinitFrame {
        x: rect.x as f32,
        y: rect.y as f32,
        w: rect.w as f32,
        h: rect.h as f32,
    }
}

/// A pure, renderer-free description of where a winit window belongs.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub struct WinitFrame {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Apply a drag delta: anchored sides absorb the motion. Center-horizontal
/// uses both sides inversely so the surface stays between them.
pub fn drag(margin: (i32, i32, i32, i32), cfg: &Config, dx: i32, dy: i32) -> (i32, i32, i32, i32) {
    let (mut top, mut right, mut bottom, mut left) = margin;
    match cfg.horizontal {
        HorizontalAnchor::Left => left = clamp(left + dx),
        HorizontalAnchor::Right => right = clamp(right - dx),
        HorizontalAnchor::Center => {
            left = clamp(left + dx);
            right = clamp(right - dx);
        }
    }
    match effective_vertical(cfg) {
        VerticalAnchor::Top => top = clamp(top + dy),
        VerticalAnchor::Bottom => bottom = clamp(bottom - dy),
    }
    (top, right, bottom, left)
}

#[cfg(test)]
mod tests {
    use hyprlay_core::compositor::macos_flip_y;
    use hyprlay_core::compositor::physical_to_logical;

    use super::*;

    fn cfg(h: HorizontalAnchor, v: VerticalAnchor) -> Config {
        Config {
            horizontal: h,
            vertical: v,
            offset_x: 16,
            offset_y: 24,
            ..Config::default()
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn anchor_combines_configured_edges() {
        use iced_layershell::reexport::Anchor;
        assert_eq!(
            anchor(&cfg(HorizontalAnchor::Left, VerticalAnchor::Top)),
            Anchor::Top | Anchor::Left
        );
        assert_eq!(
            anchor(&cfg(HorizontalAnchor::Right, VerticalAnchor::Bottom)),
            Anchor::Bottom | Anchor::Right
        );
        assert_eq!(
            anchor(&cfg(HorizontalAnchor::Center, VerticalAnchor::Top)),
            Anchor::Top | Anchor::Left | Anchor::Right
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn anchor_override_decides_the_glue_edge() {
        use iced_layershell::reexport::Anchor;
        // Auto follows the position's vertical edge...
        assert!(anchor(&cfg(HorizontalAnchor::Left, VerticalAnchor::Top)).contains(Anchor::Top));
        // ...an explicit anchor overrides it.
        let mut c = cfg(HorizontalAnchor::Left, VerticalAnchor::Top);
        c.anchor = hyprlay_core::config::AnchorMode::Bottom;
        let anchored_bottom = anchor(&c);
        assert!(anchored_bottom.contains(Anchor::Bottom));
        assert!(!anchored_bottom.contains(Anchor::Top));
    }

    #[test]
    fn initial_margin_sets_all_sides_from_config() {
        assert_eq!(
            offset(&cfg(HorizontalAnchor::Left, VerticalAnchor::Top)),
            (24, 16, 24, 16)
        );
    }

    #[test]
    fn clamp_bounds_drag_accumulation() {
        assert_eq!(clamp(9_000_000), 8000);
        assert_eq!(clamp(-9_000_000), -8000);
        assert_eq!(clamp(120), 120);
    }

    #[test]
    fn drag_left_anchored_grows_left_margin() {
        let m = offset(&cfg(HorizontalAnchor::Left, VerticalAnchor::Top));
        // dy=−20 → top−20; dx=+30 → left+30.
        assert_eq!(
            drag(
                m,
                &cfg(HorizontalAnchor::Left, VerticalAnchor::Top),
                30,
                -20
            ),
            (4, 16, 24, 46)
        );
    }

    #[test]
    fn drag_right_anchored_grows_right_margin() {
        let c = cfg(HorizontalAnchor::Right, VerticalAnchor::Top);
        assert_eq!(drag(offset(&c), &c, 30, -20), (4, -14, 24, 16));
    }

    #[test]
    fn drag_center_anchored_moves_both_horizontal_margins() {
        let c = cfg(HorizontalAnchor::Center, VerticalAnchor::Top);
        assert_eq!(drag(offset(&c), &c, 30, -20), (4, -14, 24, 46));
    }

    #[test]
    fn drag_bottom_anchored_moves_away_when_dragged_up() {
        let c = cfg(HorizontalAnchor::Left, VerticalAnchor::Bottom);
        // Dragging up (dy < 0) pulls the surface away from the bottom edge.
        assert_eq!(drag(offset(&c), &c, 0, -40).2, 64);
    }

    #[test]
    fn drag_accumulates_on_the_anchored_edge_when_anchor_overrides_position() {
        // Top position but bottom-glued (anchor=bottom): dy must accumulate
        // on the BOTTOM margin — the edge the surface actually hangs from —
        // or offsets fight the anchor.
        let mut c = cfg(HorizontalAnchor::Left, VerticalAnchor::Top);
        c.anchor = hyprlay_core::config::AnchorMode::Bottom;
        let dragged = drag(offset(&c), &c, 0, -40);
        assert_eq!(dragged.2, 64, "bottom margin absorbs the upward drag");
        assert_eq!(dragged.0, 24, "top margin untouched");
    }

    #[test]
    fn auto_still_uses_vertical_for_drag() {
        // Auto keeps following the position's vertical side in both
        // directions.
        let top = cfg(HorizontalAnchor::Left, VerticalAnchor::Top);
        assert_eq!(drag(offset(&top), &top, 0, -40).0, -16);
        let bottom = cfg(HorizontalAnchor::Left, VerticalAnchor::Bottom);
        assert_eq!(drag(offset(&bottom), &bottom, 0, -40).2, 64);
    }

    fn monitor(x: i32, y: i32, w: i32, h: i32) -> hyprlay_core::compositor::Monitor {
        hyprlay_core::compositor::Monitor {
            name: "test".to_string(),
            description: String::new(),
            active: true,
            x,
            y,
            width: w,
            height: h,
            scale: 1.0,
        }
    }

    fn monitor_scaled(
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        scale: f32,
    ) -> hyprlay_core::compositor::Monitor {
        hyprlay_core::compositor::Monitor {
            name: "test".to_string(),
            description: String::new(),
            active: true,
            x,
            y,
            width: w,
            height: h,
            scale,
        }
    }

    #[test]
    fn overlay_rect_left_top_uses_offset_and_monitor_origin() {
        let cfg = cfg(HorizontalAnchor::Left, VerticalAnchor::Top);
        let m = monitor(10, 20, 1920, 1080);
        let rect = overlay_rect(&cfg, (300, 100), (24, 16, 24, 16), Some(&m));
        assert_eq!(
            rect,
            Rect {
                x: 26,
                y: 44,
                w: 300,
                h: 100
            }
        );
        assert!(rect.contains((30, 50)));
        assert!(!rect.contains((0, 0)));
    }

    #[test]
    fn overlay_rect_right_bottom_uses_monitor_size_minus_offset() {
        let cfg = cfg(HorizontalAnchor::Right, VerticalAnchor::Bottom);
        let m = monitor(0, 0, 1920, 1080);
        let rect = overlay_rect(&cfg, (300, 100), (24, 16, 24, 16), Some(&m));
        assert_eq!(rect.x, 1920 - 300 - 16);
        assert_eq!(rect.y, 1080 - 100 - 24);
    }

    #[test]
    fn overlay_rect_center_top_is_centered_horizontally() {
        let cfg = cfg(HorizontalAnchor::Center, VerticalAnchor::Top);
        let m = monitor(0, 0, 1920, 1080);
        let rect = overlay_rect(&cfg, (300, 100), (24, 16, 24, 16), Some(&m));
        assert_eq!(rect.x, (1920 - 300) / 2);
        assert_eq!(rect.y, 24);
    }

    #[test]
    fn overlay_rect_center_with_drag_offsets_shifts_from_center() {
        let cfg = cfg(HorizontalAnchor::Center, VerticalAnchor::Top);
        let m = monitor(0, 0, 1920, 1080);
        // Simulate drag 30px right: left 46, right -14
        let offset = (24, -14, 24, 46);
        let rect = overlay_rect(&cfg, (300, 100), offset, Some(&m));
        assert_eq!(rect.x, (1920 - 300) / 2 + (46 - (-14)) / 2);
    }

    #[test]
    fn overlay_rect_monitor_offset_shifts_global_coords() {
        let cfg = cfg(HorizontalAnchor::Left, VerticalAnchor::Top);
        let m = monitor(1920, 0, 1920, 1080);
        let rect = overlay_rect(&cfg, (300, 100), (10, 5, 10, 5), Some(&m));
        assert_eq!(rect.x, 1925);
        assert_eq!(rect.y, 10);
        assert!(rect.contains((1930, 20)));
        assert!(!rect.contains((10, 20)));
    }

    #[test]
    fn overlay_rect_fallback_when_monitor_none_uses_left_top() {
        let cfg = cfg(HorizontalAnchor::Right, VerticalAnchor::Bottom);
        let rect = overlay_rect(&cfg, (300, 100), (24, 16, 24, 16), None);
        assert_eq!(rect.x, 16);
        assert_eq!(rect.y, 24);
    }

    #[test]
    fn rect_contains_is_inclusive_top_left_exclusive_bottom_right() {
        let r = Rect {
            x: 10,
            y: 20,
            w: 100,
            h: 50,
        };
        assert!(r.contains((10, 20)));
        assert!(r.contains((109, 69)));
        assert!(!r.contains((110, 20)));
        assert!(!r.contains((10, 70)));
        assert!(!r.contains((9, 20)));
    }

    #[test]
    fn overlay_rect_accounts_for_monitor_scale() {
        // eDP-1 1920x1200 @ 1.25 → logical 1536x960, top-right 300w, offset 12
        // x = 1536 - 300 - 12 = 1224, y = 12
        let c = cfg(HorizontalAnchor::Right, VerticalAnchor::Top);
        let m = monitor_scaled(0, 0, 1920, 1200, 1.25);
        let rect = overlay_rect(&c, (300, 88), (12, 12, 12, 12), Some(&m));
        assert_eq!(rect.x, 1224);
        assert_eq!(rect.y, 12);
        assert_eq!(rect.w, 300);
        assert_eq!(rect.h, 88);
        // cursor inside scaled rect should be hoverable (was unreachable before: x=1608 raw)
        assert!(rect.contains((1443, 40)));
        assert!(!rect.contains((1600, 40)));
        // center with scale
        let c2 = cfg(HorizontalAnchor::Center, VerticalAnchor::Top);
        let rect_c = overlay_rect(&c2, (300, 100), (12, 12, 12, 12), Some(&m));
        assert_eq!(rect_c.x, (1536 - 300) / 2);
    }

    #[test]
    fn overlay_rect_scaled_bottom_uses_scaled_height() {
        let cfg = cfg(HorizontalAnchor::Left, VerticalAnchor::Bottom);
        let m = monitor_scaled(0, 0, 1920, 1200, 1.25);
        let rect = overlay_rect(&cfg, (300, 100), (12, 16, 12, 16), Some(&m));
        // logical height 960 → y = 960 -100 -12 =848
        assert_eq!(rect.y, 848);
    }

    #[test]
    fn pick_monitor_prefers_named_then_active() {
        let monitors = vec![
            Monitor {
                name: "test".to_string(),
                active: false,
                ..Monitor::default()
            },
            Monitor {
                name: "HDMI-A-1".to_string(),
                active: false,
                ..Monitor::default()
            },
            Monitor {
                name: "eDP-1".to_string(),
                active: true,
                ..Monitor::default()
            },
        ];
        // Always names the requested output even when it is not active.
        assert_eq!(
            pick_monitor(&monitors, Some("HDMI-A-1")).map(|m| m.name.as_str()),
            Some("HDMI-A-1")
        );
        // Unknown name falls back to the active one.
        assert_eq!(
            pick_monitor(&monitors, Some("DP-9")).map(|m| m.name.as_str()),
            Some("eDP-1")
        );
        // No name → active.
        assert_eq!(
            pick_monitor(&monitors, None).map(|m| m.name.as_str()),
            Some("eDP-1")
        );
        // Empty list → none.
        assert_eq!(pick_monitor(&[], None), None);
    }

    #[test]
    fn monitor_logical_resolves_scale_to_logical_pixels() {
        // 1920x1200 @1.25 → logical 1536x960, origin preserved.
        let m = monitor_scaled(10, 20, 1920, 1200, 1.25);
        assert_eq!(monitor_logical(&m), (10, 20, 1536, 960));
        // A 0 scale is degenerate → treated as 1.0.
        let zero = Monitor {
            scale: 0.0,
            ..monitor(0, 0, 1920, 1080)
        };
        assert_eq!(monitor_logical(&zero), (0, 0, 1920, 1080));
    }

    #[test]
    fn physical_to_logical_rounds_to_nearest_logical_pixel() {
        assert_eq!(physical_to_logical(1920, 1.25), 1536);
        assert_eq!(physical_to_logical(1920, 1.0), 1920);
        assert_eq!(physical_to_logical(1920, 0.0), 1920); // degenerate scale
        assert_eq!(physical_to_logical(-100, 2.0), -50);
    }

    #[test]
    fn macos_flip_y_converts_top_left_to_bottom_left_origin() {
        // A point 100px from the top of a 1080 logical space is 980 from the
        // bottom under the macOS Y-up convention.
        assert_eq!(macos_flip_y(100, 1080), 980);
        assert_eq!(macos_flip_y(0, 1080), 1080);
        assert_eq!(macos_flip_y(1080, 1080), 0);
    }

    #[test]
    fn winit_frame_matches_the_layershell_overlay_rect() {
        let cfg = cfg(HorizontalAnchor::Left, VerticalAnchor::Top);
        let m = monitor(10, 20, 1920, 1080);
        let frame = winit_frame(&cfg, (300, 100), (24, 16, 24, 16), Some(&m));
        assert_eq!(
            frame,
            WinitFrame {
                x: 26.0,
                y: 44.0,
                w: 300.0,
                h: 100.0
            }
        );
    }
}
