//! One-page navigation: measure the content layout, jump to a section,
//! and keep the sidebar highlight (scrollspy) on the section under the
//! viewport top. Also owns the two widget ids shared across the GUI: the
//! header's search input and the one-page content scrollable.

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

/// Measure the one-page content and report back as [`Message::Measured`].
/// Widget operations run against the layout built from the very latest
/// state, so the offsets are fresh even right after a search-clear
/// re-render or a picker expansion changed heights.
pub(super) fn measure_sections(jump: Option<Section>) -> Task<Message> {
    iced_runtime::task::widget(MeasureSections {
        jump,
        content: None,
        offsets: [0.0; Section::ALL.len()],
    })
}

/// Scroll the one-page content so `y` px into it sit at the viewport top;
/// the horizontal offset is left alone.
fn scroll_content_to(y: f32) -> Task<Message> {
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
pub(super) fn restore_scroll(gui: &Gui) -> Task<Message> {
    scroll_content_to(gui.last_scroll_y)
}

/// Geometry the measure operation learns about the one-page scrollable.
#[derive(Debug, Clone, Copy)]
struct ContentMeasure {
    viewport: Rectangle,
    content: Rectangle,
}

impl ContentMeasure {
    /// How far the content can scroll at all.
    fn max_scroll(self) -> f32 {
        (self.content.height - self.viewport.height).max(0.0)
    }
}

/// Traverses the widget tree once, recording the one-page scrollable's
/// geometry and every section header's offset within the content, then
/// delivers them as [`Message::Measured`]. `jump` rides along so the
/// receiver knows whether to scroll (navigation) or re-derive the
/// highlight (scrollspy). When the one-pager is not in the tree (the
/// search page is up), the operation produces nothing.
struct MeasureSections {
    jump: Option<Section>,
    content: Option<ContentMeasure>,
    offsets: [f32; Section::ALL.len()],
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
        _translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        if id == Some(&content_scroll_id()) {
            self.content = Some(ContentMeasure {
                viewport: bounds,
                content: content_bounds,
            });
        }
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        // The scrollable hook fires before its children are traversed, so
        // the content geometry is always known by the time a header anchor
        // is visited.
        let Some(content) = self.content else {
            return;
        };
        for (index, section) in Section::ALL.into_iter().enumerate() {
            if id == Some(&Id::new(section.anchor_id())) {
                self.offsets[index] = offset_within_content(bounds, content.content);
            }
        }
    }

    fn finish(&self) -> Outcome<Message> {
        let Some(content) = self.content else {
            return Outcome::None;
        };
        Outcome::Some(Message::Measured {
            offsets: self.offsets,
            max_scroll: content.max_scroll(),
            jump: self.jump,
        })
    }
}

#[cfg(test)]
mod tests {
    use iced::Point;

    use super::*;

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

    /// A header takes over the highlight once the viewport top is within
    /// half a header height of it — this is what keeps a landed jump (which
    /// parks the header exactly at the top) on its own section. 60 px above
    /// the Layout header the highlight must still be Position; 10 px above
    /// it, Layout already wins.
    #[test]
    fn scrollspy_flips_while_a_header_is_half_a_header_away() {
        let offsets = spy_offsets();
        assert_eq!(active_section_for(840.0, &offsets), Section::Position);
        assert_eq!(active_section_for(890.0, &offsets), Section::Layout);
    }

    /// The last section's header can never reach the viewport top (the
    /// Connection section is shorter than the viewport), so once the page
    /// is scrolled to its end the caller passes INFINITY and the highlight
    /// must clamp to Connection instead of staying on Colors.
    #[test]
    fn scrollspy_clamps_to_connection_at_the_end_of_the_page() {
        let offsets = spy_offsets();
        // Without the clamp, a bottom scroll (~2200 here) would highlight
        // Colors although the user is looking at Connection.
        assert_eq!(active_section_for(2200.0, &offsets), Section::Colors);
        assert_eq!(
            active_section_for(f32::INFINITY, &offsets),
            Section::Connection
        );
    }

    /// A page could in principle start with a tall top padding; before the
    /// first measurement lands or before any header qualifies, the
    /// highlight must fall back to the first section, never panic or wrap.
    #[test]
    fn scrollspy_falls_back_to_the_first_section_at_the_top() {
        let offsets = [100.0, 900.0, 1500.0, 2100.0, 2600.0];
        assert_eq!(active_section_for(0.0, &offsets), Section::Position);
    }

    /// Header offsets are read from window-space layout bounds while the
    /// page may be scrolled; the jump math depends on the difference
    /// between the two rects being independent of that translation.
    #[test]
    fn header_offset_is_its_distance_below_the_content_and_ignores_scroll() {
        let content = Rectangle::new(Point::ORIGIN, iced::Size::new(600.0, 4000.0));
        let header = Rectangle::new(Point::new(16.0, 908.0), iced::Size::new(568.0, 30.0));
        assert_eq!(offset_within_content(header, content), 908.0);

        // The same layout scrolled down by 250 px: both rects move, the
        // offset within the content must not.
        let scrolled_content = Rectangle::new(Point::new(16.0, -250.0), content.size());
        let scrolled_header = Rectangle::new(Point::new(32.0, 658.0), header.size());
        assert_eq!(
            offset_within_content(scrolled_header, scrolled_content),
            908.0
        );
    }
}
