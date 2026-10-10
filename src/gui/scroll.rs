//! One-page navigation: measure the content layout, jump to a section,
//! and keep the sidebar highlight (scrollspy) on the section under the
//! viewport top. Also owns the two widget ids shared across the GUI: the
//! header's search input and the one-page content scrollable.

use std::collections::HashMap;

use iced::Rectangle;
use iced::Task;
use iced::Vector;
use iced::widget::Id;
use iced_runtime::core::widget::Operation;
use iced_runtime::core::widget::operation::Outcome;
use iced_runtime::core::widget::operation::scrollable::Scrollable;

use super::Gui;
use super::Message;
use super::fields::CONTENT_SCROLL_ID;
use super::fields::SEARCH_ID;
use super::fields::Section;

pub(super) fn widget_id() -> iced::widget::Id {
    iced::widget::Id::new(SEARCH_ID)
}

/// Id of the one-page content scrollable — the jump target for navigation
/// and the widget the measure operation reads geometry from.
fn content_scroll_id() -> Id {
    Id::new(CONTENT_SCROLL_ID)
}

/// Half a section-header height: how close a header must be to the top of
/// the viewport before the scrollspy names its section. Small enough that
/// the highlight only moves once a header actually arrives, big enough
/// that a landed jump — which parks the header exactly at the top — keeps
/// its own highlight.
const SECTION_EPSILON: f32 = 16.0;

/// Slack around the measured maximum scroll within which the page counts
/// as scrolled to its end.
pub(super) const BOTTOM_SLACK: f32 = 1.0;

/// The section whose header sits at or above `scroll_y` — the sidebar
/// highlight for that scroll position: the last section whose measured
/// header offset is within `scroll_y + SECTION_EPSILON`. The caller
/// passes [`f32::INFINITY`] once the page has scrolled to its end,
/// because the last header can never reach the viewport top itself.
/// `offsets` must be the headers measured in [`Section::ALL`] order —
/// one offset per section, at the same index.
pub(super) fn active_section_for(scroll_y: f32, offsets: &[f32; Section::ALL.len()]) -> Section {
    let mut active = Section::ALL[0];
    for (index, offset) in offsets.iter().enumerate() {
        if *offset > scroll_y + SECTION_EPSILON {
            break;
        }
        active = Section::ALL[index];
    }
    active
}

/// Where `header` sits inside the scrollable content, in px below the
/// content's top. Both rects are window-space layout bounds, so the
/// current scroll translation appears in both and cancels out.
fn offset_within_content(header_bounds: Rectangle, content_bounds: Rectangle) -> f32 {
    header_bounds.y - content_bounds.y
}

/// Slack around a row's bounds within which it counts as already on screen:
/// a sub-pixel rounding difference must not make the page jump.
const VISIBLE_SLACK: f32 = 2.0;

/// How far above the viewport top a revealed row is parked, so its ring is not
/// flush against the edge.
const REVEAL_MARGIN: f32 = 8.0;

/// Where to scroll so the row at `offset` (`height` tall) is inside the
/// viewport showing content from `scroll_y` for `viewport_h` px, or `None`
/// when it already is. Keyboard focus can walk 30 rows down a one-page
/// scroll, so tabbing has to bring the focused row along -- but a row already
/// in view must not move the page under the pointer.
fn reveal_scroll(scroll_y: f32, viewport_h: f32, offset: f32, height: f32) -> Option<f32> {
    let in_view = offset >= scroll_y - VISIBLE_SLACK
        && offset + height <= scroll_y + viewport_h + VISIBLE_SLACK;
    (!in_view).then(|| (offset - REVEAL_MARGIN).max(0.0))
}

/// What the next measure pass should scroll for.
#[derive(Debug, Clone)]
pub(super) enum Jump {
    Section(Section),
    /// A row, by the widget id it is tagged with (`fields::row_id`).
    Row(Id),
}

/// Measure the one-page content and report back as [`Message::Measured`].
/// Widget operations run against the layout built from the very latest
/// state, so the offsets are fresh even right after a search-clear
/// re-render or a picker expansion changed heights.
pub(super) fn measure_sections(jump: Option<Jump>) -> Task<Message> {
    iced_runtime::task::widget(MeasureSections {
        jump,
        content: None,
        offsets: [0.0; Section::ALL.len()],
        anchored: false,
        rows: HashMap::new(),
    })
}

/// Scroll the one-page content so `y` px into it sit at the viewport top;
/// the horizontal offset is left alone.
pub(super) fn scroll_content_to(y: f32) -> Task<Message> {
    iced_runtime::widget::operation::scroll_to(
        content_scroll_id(),
        iced::widget::scrollable::AbsoluteOffset {
            x: None,
            y: Some(y),
        },
    )
}

/// Scroll the one-pager so `section`'s header sits at the top of the
/// viewport. `offsets` must come from a fresh measurement (see
/// [`measure_sections`]); out-of-range targets clamp inside the
/// scrollable.
pub(super) fn scroll_to_section(
    section: Section,
    offsets: [f32; Section::ALL.len()],
) -> Task<Message> {
    scroll_content_to(offsets[section.index()])
}

/// D4: land the one-pager back on the offset tracked before the search
/// page replaced it. While the search page was up, nothing reported
/// Scrolled, so [`Gui::last_scroll_y`] still holds that position.
///
/// The pass behind the scroll is what re-derives the sidebar highlight for
/// that offset. It belongs here and not on [`scroll_content_to`], which the
/// field reveal also uses: a reveal target can sit past the end of the page,
/// where the scrollable clamps it and the bottom clamp reads as "scrolled to
/// the end" — Connection, the one section with no config group. A restore
/// target is an offset the page was already at, so the answer is read, not
/// clamped.
pub(super) fn restore_scroll(gui: &Gui) -> Task<Message> {
    scroll_content_to(gui.last_scroll_y).chain(measure_sections(None))
}

/// Geometry the measure operation learns about the one-page scrollable.
#[derive(Debug, Clone, Copy)]
struct ContentMeasure {
    viewport: Rectangle,
    content: Rectangle,
    /// How far the content is scrolled now, in px from its top. Only the
    /// translation says so: iced hands the hook the viewport and content
    /// rects as laid out and applies the scroll when it draws, so the two
    /// rects keep the same `y` however far the page is scrolled.
    scroll_y: f32,
}

impl ContentMeasure {
    /// How far the content can scroll at all.
    fn max_scroll(self) -> f32 {
        (self.content.height - self.viewport.height).max(0.0)
    }
}

/// Traverses the widget tree once, recording the one-page scrollable's
/// geometry, every section header's offset within the content, and every
/// focusable field row's offset and height, then delivers them as
/// [`Message::Measured`] — or, for a field jump that is not already on
/// screen, as [`Message::ScrollContentTo`]. The search page rides the same
/// scrollable but renders no section headers, so a pass that saw none reports
/// nothing at all: five unmeasured offsets stay at zero, and the scrollspy
/// would walk all of them to Connection whatever the scroll position is.
struct MeasureSections {
    jump: Option<Jump>,
    content: Option<ContentMeasure>,
    offsets: [f32; Section::ALL.len()],
    /// Whether a section header's anchor was visited, i.e. whether `offsets`
    /// means anything.
    anchored: bool,
    /// Geometry of every id-carrying container, keyed by that id. A row's id
    /// comes from `fields::row_id`, so a keyed row and a keyless one are
    /// measured the same way and `Jump::Row` needs no per-variant branch.
    rows: HashMap<Id, (f32, f32)>,
}

impl Operation<Message> for MeasureSections {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Message>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        if id == Some(&content_scroll_id()) {
            self.content = Some(ContentMeasure {
                viewport: bounds,
                content: content_bounds,
                // `translation` is +offset for the default top anchor
                // (`scrollable.rs`: `Offset::translation` returns the
                // absolute offset under `Anchor::Start`), so it is the scroll
                // position with no sign flip. Two things make that true here:
                // nothing in `src/` sets `.anchor_y`/`.anchor` — a future
                // `Anchor::End` would invert it silently — and `Offset::absolute`
                // clamps, so this is the real position rather than a request.
                // Deriving it from the two rects instead yields a constant 0.0,
                // which is what made every reveal past the first
                // viewport-height scroll. The tests below hand this hook a
                // fabricated `Vector`, so this comment is the only evidence
                // for the sign.
                scroll_y: translation.y,
            });
        }
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        // The scrollable hook fires before its children are traversed, so the
        // content geometry is always known by the time an anchor is visited.
        let Some(content) = self.content else {
            return;
        };
        let Some(id) = id else {
            return;
        };
        for (index, section) in Section::ALL.into_iter().enumerate() {
            if id == &Id::new(section.anchor_id()) {
                self.anchored = true;
                self.offsets[index] = offset_within_content(bounds, content.content);
            }
        }
        self.rows.insert(
            id.clone(),
            (
                offset_within_content(bounds, content.content),
                bounds.height,
            ),
        );
    }

    fn finish(&self) -> Outcome<Message> {
        let Some(content) = self.content else {
            return Outcome::None;
        };
        // A row jump answers on its own instead of through Measured: the row
        // may already be on screen, and then nothing at all should move.
        if let Some(Jump::Row(row)) = &self.jump {
            let target = self.rows.get(row).and_then(|(offset, height)| {
                reveal_scroll(content.scroll_y, content.viewport.height, *offset, *height)
            });
            return match target {
                Some(y) => Outcome::Some(Message::ScrollContentTo(y)),
                None => Outcome::None,
            };
        }
        // `offsets` is only measured if a header was there to measure it from,
        // and a search page has none. Reporting the initialiser instead would
        // hand the scrollspy five zeroes, which it reads as "scrolled past
        // every header" and answers Connection — the one section with no
        // config group, so Ctrl+R would then reset nothing.
        if !self.anchored {
            return Outcome::None;
        }
        Outcome::Some(Message::Measured {
            offsets: self.offsets,
            max_scroll: content.max_scroll(),
            jump: match self.jump {
                Some(Jump::Section(section)) => Some(section),
                _ => None,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use iced::Point;
    use iced::Size;

    use super::*;

    /// Stands in for the scrollable's own state: the measure operation only
    /// ever reads geometry and the translation, never drives the scroll.
    struct Unused;

    impl Scrollable for Unused {
        fn snap_to(&mut self, _offset: iced::widget::scrollable::RelativeOffset<Option<f32>>) {}
        fn scroll_to(&mut self, _offset: iced::widget::scrollable::AbsoluteOffset<Option<f32>>) {}
        fn scroll_by(
            &mut self,
            _offset: iced::widget::scrollable::AbsoluteOffset,
            _bounds: Rectangle,
            _content_bounds: Rectangle,
        ) {
        }
    }

    /// The widget id the fixture row is tagged with. Any `&'static str` would
    /// do — the reveal looks a row up by the id `fields::row_id` gave it, so
    /// using a real key name here also keeps that path honest.
    const FIXTURE_ROW: &str = "max-rows";

    /// A measure pass over a page scrolled 700 px down, aimed at one row the
    /// caller then places with [`record_row`]. `reveal_scroll` turns that
    /// geometry into the offset the page must jump to.
    fn measure_at_700() -> MeasureSections {
        let mut op = MeasureSections {
            jump: Some(Jump::Row(Id::new(FIXTURE_ROW))),
            content: None,
            offsets: [0.0; Section::ALL.len()],
            anchored: true,
            rows: HashMap::new(),
        };
        op.scrollable(
            Some(&Id::new(CONTENT_SCROLL_ID)),
            Rectangle::new(Point::ORIGIN, Size::new(600.0, 500.0)),
            Rectangle::new(Point::ORIGIN, Size::new(600.0, 4000.0)),
            Vector::new(0.0, 700.0),
            &mut Unused,
        );
        op
    }

    /// Place the fixture row at `offset` px below the content's top, `height`
    /// tall — through the traversal hook rather than the map, so the recording
    /// is the code the app runs.
    fn record_row(op: &mut MeasureSections, offset: f32, height: f32) {
        op.container(
            Some(&Id::new(FIXTURE_ROW)),
            Rectangle::new(Point::new(0.0, offset), Size::new(600.0, height)),
        );
    }

    /// The reveal offset `op` decided on, or `None` when it decided to leave
    /// the page alone. `Outcome` is not `PartialEq`, and only a row jump
    /// answers with a scroll.
    fn reveal(op: &MeasureSections) -> Option<f32> {
        match op.finish() {
            Outcome::Some(Message::ScrollContentTo(y)) => Some(y),
            Outcome::None => None,
            other => panic!("a row jump answers with a scroll or nothing, got {other:?}"),
        }
    }

    /// The scroll position is the translation the hook is handed, because iced
    /// never translates the two layout rects on scroll: they carry the same
    /// `y` at every offset, so any derivation from their difference reads
    /// 0.0 forever. That constant is what left Shift+Tab revealing nothing.
    #[test]
    fn the_scroll_offset_is_the_translation_not_the_two_rects() {
        let op = measure_at_700();
        assert_eq!(op.content.expect("content measured").scroll_y, 700.0);
    }

    /// At 700 px, a row 400 px above the viewport must be revealed by
    /// scrolling back up to it. Judged against a scroll position of 0.0 — the
    /// value the rect derivation produces — that same row reads as "already in
    /// view" and the ring walks off-screen with the page staying put.
    #[test]
    fn a_row_above_a_scrolled_page_is_revealed() {
        let mut op = measure_at_700();
        record_row(&mut op, 300.0, 60.0);
        assert_eq!(reveal(&op), Some(292.0));
    }

    /// And the mirror: a row the page has not reached yet still scrolls down.
    #[test]
    fn a_row_below_a_scrolled_page_is_revealed_too() {
        let mut op = measure_at_700();
        record_row(&mut op, 3000.0, 60.0);
        assert_eq!(reveal(&op), Some(2992.0));
    }

    /// A row already inside the scrolled viewport is left alone, so tabbing
    /// through one section walks the ring and not the page.
    #[test]
    fn a_row_inside_a_scrolled_viewport_is_left_alone() {
        let mut op = measure_at_700();
        record_row(&mut op, 900.0, 60.0);
        assert_eq!(reveal(&op), None);
    }

    /// A focus jump for a row this pass never measured — the search page
    /// dropped it, so no anchor carries its id — reveals nothing rather than
    /// scrolling to a stale offset.
    #[test]
    fn a_field_the_page_did_not_render_reveals_nothing() {
        let op = measure_at_700();
        assert_eq!(reveal(&op), None);
    }

    /// The two credential rows have no config `Key`, so they are reached by
    /// their own widget ids — which is exactly why the map is keyed by id
    /// rather than by `Key::ALL`. Without this the reveal would be a
    /// `Key`-indexed array that a keyless row cannot appear in, and Tab onto
    /// the credentials would ring a row the page never scrolls to.
    #[test]
    fn a_keyless_row_is_revealed_by_its_own_widget_id() {
        let mut op = MeasureSections {
            jump: Some(Jump::Row(Id::new("row-client-id"))),
            content: None,
            offsets: [0.0; Section::ALL.len()],
            anchored: true,
            rows: HashMap::new(),
        };
        op.scrollable(
            Some(&Id::new(CONTENT_SCROLL_ID)),
            Rectangle::new(Point::ORIGIN, Size::new(600.0, 500.0)),
            Rectangle::new(Point::ORIGIN, Size::new(600.0, 4000.0)),
            Vector::new(0.0, 700.0),
            &mut Unused,
        );
        op.container(
            Some(&Id::new("row-client-id")),
            Rectangle::new(Point::new(0.0, 3600.0), Size::new(600.0, 40.0)),
        );
        assert_eq!(reveal(&op), Some(3592.0));
    }

    /// A measure pass over a page that carries the content scrollable but no
    /// section anchors — the search page, which needs the id so a Tab reveal
    /// finds a scrollable to move.
    fn measure_a_page_with_no_anchors() -> MeasureSections {
        let mut op = MeasureSections {
            jump: None,
            content: None,
            offsets: [0.0; Section::ALL.len()],
            anchored: false,
            rows: HashMap::new(),
        };
        op.scrollable(
            Some(&Id::new(CONTENT_SCROLL_ID)),
            Rectangle::new(Point::ORIGIN, Size::new(600.0, 500.0)),
            Rectangle::new(Point::ORIGIN, Size::new(600.0, 4000.0)),
            Vector::new(0.0, 700.0),
            &mut Unused,
        );
        op
    }

    /// Such a page reports no offsets. Five unmeasured ones stay at their zero
    /// initialiser, and the scrollspy then names the last section at *every*
    /// scroll position — which is also the section Ctrl+R has no config group
    /// for, so Ctrl+R silently stops resetting anything.
    #[test]
    fn a_page_with_no_section_anchors_measures_nothing() {
        assert!(
            matches!(measure_a_page_with_no_anchors().finish(), Outcome::None),
            "offsets nothing measured are five zeroes, and active_section_for \
             walks all five to Connection"
        );
    }

    /// The mirror, so the guard above cannot be widened past a real page: one
    /// visited anchor is enough to report, and its measured offset is the one
    /// that comes back.
    #[test]
    fn a_page_that_rendered_a_section_anchor_reports_its_offsets() {
        let mut op = measure_a_page_with_no_anchors();
        op.container(
            Some(&Id::new(Section::Layout.anchor_id())),
            Rectangle::new(Point::new(16.0, 908.0), Size::new(568.0, 30.0)),
        );
        let Outcome::Some(Message::Measured { offsets, .. }) = op.finish() else {
            panic!("the one-pager renders anchors, so it must report offsets");
        };
        assert_eq!(offsets[Section::Layout.index()], 908.0);
        assert_eq!(offsets[Section::Colors.index()], 0.0, "unvisited stay zero");
    }

    /// Realistic header offsets for the one-page view: five sections
    /// stacked downward, the first header just below the page's top
    /// padding, later ones several hundred px apart.
    fn spy_offsets() -> [f32; Section::ALL.len()] {
        [8.0, 900.0, 1500.0, 2100.0, 2600.0]
    }

    /// At the top of the page the first header is inside the epsilon, so
    /// Position is highlighted; halfway into a section the highlight still
    /// names that section.
    #[test]
    fn scrollspy_names_the_section_under_the_top_of_the_viewport() {
        let offsets = spy_offsets();
        assert_eq!(active_section_for(0.0, &offsets), Section::Position);
        assert_eq!(active_section_for(1000.0, &offsets), Section::Layout);
        assert_eq!(active_section_for(1900.0, &offsets), Section::Opacity);
        assert_eq!(active_section_for(2400.0, &offsets), Section::Colors);
    }

    /// A header takes over within half a header height — this is what keeps a
    /// landed jump, which parks the header exactly at the top, on its own
    /// section.
    #[test]
    fn scrollspy_flips_while_a_header_is_half_a_header_away() {
        let offsets = spy_offsets();
        assert_eq!(active_section_for(840.0, &offsets), Section::Position);
        assert_eq!(active_section_for(890.0, &offsets), Section::Layout);
    }

    /// A row taller than the viewport can never fit, so it scrolls to its top
    /// rather than being treated as visible and stranded.
    #[test]
    fn a_row_taller_than_the_viewport_scrolls_to_its_top() {
        assert_eq!(reveal_scroll(0.0, 500.0, 100.0, 600.0), Some(92.0));
    }

    /// One slack-width past either edge still counts as visible; one pixel
    /// further out does not.
    #[test]
    fn the_visibility_slack_decides_the_boundary() {
        let viewport = 500.0;
        assert_eq!(
            reveal_scroll(0.0, viewport, 442.0, 60.0),
            None,
            "a row ending exactly VISIBLE_SLACK past the viewport is still in view"
        );
        assert_eq!(
            reveal_scroll(0.0, viewport, 443.0, 60.0),
            Some(435.0),
            "one pixel more and the page scrolls"
        );
        assert_eq!(
            reveal_scroll(700.0, viewport, 698.0, 60.0),
            None,
            "a row starting exactly VISIBLE_SLACK above the viewport is still in view"
        );
        assert_eq!(
            reveal_scroll(700.0, viewport, 697.0, 60.0),
            Some(689.0),
            "one pixel higher and the page scrolls back up to it"
        );
    }

    /// The reveal offset is the row's top minus the margin, clamped at zero.
    #[test]
    fn a_reveal_near_the_content_top_clamps_to_zero() {
        assert_eq!(reveal_scroll(0.0, 500.0, 4.0, 600.0), Some(0.0));
        assert_eq!(reveal_scroll(0.0, 500.0, 8.0, 600.0), Some(0.0));
        assert_eq!(reveal_scroll(0.0, 500.0, 9.0, 600.0), Some(1.0));
    }

    /// The last section's header can never reach the viewport top (Connection
    /// is shorter than the viewport), so once the page is scrolled to its end
    /// the caller passes INFINITY and the highlight must clamp to Connection.
    #[test]
    fn scrollspy_clamps_to_connection_at_the_end_of_the_page() {
        let offsets = spy_offsets();
        assert_eq!(active_section_for(2200.0, &offsets), Section::Colors);
        assert_eq!(
            active_section_for(f32::INFINITY, &offsets),
            Section::Connection
        );
    }

    /// Before the first measurement lands, the highlight falls back to the
    /// first section rather than panicking or wrapping.
    #[test]
    fn scrollspy_falls_back_to_the_first_section_at_the_top() {
        let offsets = [100.0, 900.0, 1500.0, 2100.0, 2600.0];
        assert_eq!(active_section_for(0.0, &offsets), Section::Position);
    }

    /// Header offsets come from window-space layout bounds, so the offset
    /// within the content has to be the two rects' *difference*: iced hands the
    /// hook both un-translated (the scroll lives in `translation`, see
    /// `ContentMeasure::scroll_y`), and any shift they do pick up has to cancel.
    #[test]
    fn header_offset_is_its_distance_below_the_content() {
        let content = Rectangle::new(Point::ORIGIN, iced::Size::new(600.0, 4000.0));
        let header = Rectangle::new(Point::new(16.0, 908.0), iced::Size::new(568.0, 30.0));
        assert_eq!(offset_within_content(header, content), 908.0);

        // Both rects shifted by the same 250 px: the offset within the content
        // must not move with them.
        let shifted_content = Rectangle::new(Point::new(16.0, -250.0), content.size());
        let shifted_header = Rectangle::new(Point::new(32.0, 658.0), header.size());
        assert_eq!(
            offset_within_content(shifted_header, shifted_content),
            908.0
        );
    }
}
