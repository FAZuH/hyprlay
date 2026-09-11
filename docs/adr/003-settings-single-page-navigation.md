# ADR-003: Settings GUI is one scrollable page with anchor navigation

## Status
Accepted

## Date
2026-08-31

## Context

The settings GUI (`hyprlay-gui`, `src/gui/`) shows one section at a time.
The sidebar sends `Message::Section(Section)` (`src/gui/view.rs`, `sidebar`),
and `section_page(gui, section)` (`src/gui/fields.rs`) renders that section
alone inside its own scrollable. Moving between sections means a full page
swap, so the user cannot see Position and Layout together or compare values
across sections. The GUI owns no settings logic — every change stays a
`Command` over the control socket (`src/gui/mod.rs` module doc), so this is
a view and navigation change only.

The GUI runs on iced 0.14. Before deciding, we checked the vendored iced
sources for the needed mechanics:

- `iced::widget::scrollable` supports `.id(...)` and
  `.on_scroll(|Viewport| Message)`; `Viewport::absolute_offset()` gives the
  live pixel scroll offset.
- `iced_runtime::widget::operation::scrollable::scroll_to(id, offset)`
  scrolls to a pixel offset; out-of-range y clamps internally.
- A custom `Operation` can measure layout: in the scrollable's `operate`
  hook, `content_bounds` and child bounds are both window-space, so
  `child.bounds.y - content.bounds.y` gives a child's offset inside the
  content, independent of the current scroll.
- `iced_runtime::task::widget(operation)` runs a custom operation and
  delivers its `finish()` output as a message. The repo already uses a
  runtime operation this way (`iced_runtime::widget::operation::focus` in
  `src/gui/update.rs`).

The repo uses this same widget-id mechanism for the search box
(`SEARCH_ID` in `src/gui/fields.rs`).

## Decision

- The content area becomes **one scrollable page**: `settings_page(gui)` in
  `src/gui/fields.rs` stacks all sections in `Section::ALL` order, each with
  its existing header (including the per-section reset button) and fields.
  The per-section scrollable goes away; one outer scrollable carries a fixed
  id, `CONTENT_SCROLL_ID`.
- Sidebar buttons stop switching pages. `Message::Section` renames to
  `Message::Navigate(Section)`; a click (or Ctrl+1..5) scrolls the page to
  that section's header anchor. Each header sits in a container that carries
  a widget id derived from the section (`section_anchor_id` in
  `src/gui/fields.rs`).
- A **scrollspy** keeps the sidebar highlight on the section under the top of
  the viewport. `.on_scroll` reports the offset into a `Scrolled(f32)`
  message; a custom measure `Operation` re-reads the header offsets plus
  `max_scroll`, and the pure helper `active_section_for(scroll_y, offsets)`
  picks the section. The helper cannot see the page bottom from its
  signature, so the `Measured` caller applies the bottom clamp: when
  `scroll_y >= max_scroll - BOTTOM_SLACK` (a small constant), it passes
  `f32::INFINITY` to the helper, which pins the highlight to the last
  section (Connection). A page that does not scroll emits no scroll events,
  so the `max_scroll == 0` end branch never runs in practice. The tracked
  offsets live in `Gui` as `section_offsets` and `last_scroll_y`
  (`src/gui/mod.rs`).
- Every jump measures first, then scrolls: `Navigate` chains a measure task,
  and the resulting `Measured` message stores fresh offsets and issues
  `scrollable::scroll_to(CONTENT_SCROLL_ID, ...)`. Fresh offsets make the
  jump correct even after a re-render or a picker expansion changed heights.
- Search keeps its **flat results page** (D1): typing swaps the content area
  for `search_page`; clearing it returns to the one-page view. A sidebar
  click or Ctrl+1..5 with a query set clears the search first, then jumps
  (D3).
- Clearing the search restores the scroll position held before the search
  began (D4). While the search page is up, the one-page scrollable does not
  exist, so no `Scrolled` traffic fires and `last_scroll_y` freezes at its
  pre-search value. The restore is a single `scroll_to` with that value.
- Unchanged by design: per-section reset buttons and Ctrl+R, the Connection
  apply button, header, status bar, daemon toggle, command pipeline, and
  `revert_commands`. `Section::group()` and the reset machinery stay as-is.

## Rejected alternatives

### Filter-in-place search

The search box could narrow the one page instead of swapping to a results
page. Rejected because the flat results page already exists and shows each
hit with its section name, and in-place filtering fights the scrollspy
(collapsing sections shifts every measured offset mid-typing).

### Keep per-section pages, add prev/next buttons

Preserves the current `section_page` but never lets the user see two
sections at once, which is the point of the rework. It also keeps the
page-swap message plumbing this decision removes.

### Scrollspy via polling instead of `on_scroll` + measure

A subscription or timer could poll the scroll offset. That adds a background
task and still needs the same measure operation to read layout. `on_scroll`
fires exactly when the offset changes, so polling is strictly more work.

## Consequences

- Ctrl+R resets the section the user is looking at, because `Gui.section`
  now tracks the scrollspy highlight (D2).
- The message flow has no feedback loop: `Scrolled → Measured` only sets the
  highlight and never scrolls. The derivation includes the bottom clamp, so
  the highlight reaches the last section when the page is scrolled to its
  end. `Navigate → Measured → scroll_to` fires `on_scroll` once on landing,
  which re-measures and re-derives the same section.
- The sidebar highlight stays suppressed while search text is set; the
  existing selected condition in `sidebar` (`src/gui/view.rs`) keeps its
  `gui.search.trim().is_empty()` guard.
- New state on `Gui`: `section_offsets` and `last_scroll_y`, plus the
  `CONTENT_SCROLL_ID` and per-section anchor widget ids. Two pure helpers,
  `active_section_for` and `offset_within_content`, carry the math and are
  unit-tested in `src/gui/scroll.rs`.
- Offsets measured at jump time mean stale measurements cannot send the
  viewport to the wrong place, at the cost of one measure operation per
  scroll event batch.
