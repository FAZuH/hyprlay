# DESIGN.md — hyprlay visual direction

A Discord voice-channel overlay: the roster on a transparent surface, plus a
settings window. This file records the design decisions already in the code,
so a future pass can review against a written standard instead of re-deriving
them from doc comments.

## Identity

The settings GUI deliberately mirrors Discord's dark client, so the two read
as one product. That is the anchor for the palette and the fixed dark theme —
a light mode would break the identity the product is built around.

The overlay impersonates nothing: it renders only the roster rows and the
`+N` pill, never connection or status text. An empty roster is an empty
transparent surface.

## Palette

The constants in `src/gui/theme.rs` are the single source. Three panel greys
separate the chrome by adjacency rather than by shadow:

| Constant | Value | Job |
|---|---|---|
| `HEADER_BG` / `SIDEBAR_BG` | `#17181b` / `#1a1b1e` | the two chrome strips, darkest first |
| `FIELD_BG` | `#292b33` | inputs and buttons |
| `MUTED` | `#878a94` | secondary labels; 4.78:1 on the content background |
| `BRIGHT` | `#dbdee0` | primary text |
| `ACCENT` / `ACCENT_LIT` | `#5865f2` / `#5966e6` | the Save button and the selected sidebar item only |
| `AMBER` | `#f5b83d` | the unsaved marker |
| `REPLY_GREEN` / `DANGER` | `#6bb878` / `#f24042` | the last reply, success and failure |

`ACCENT` is reserved for exactly two elements. That restraint is deliberate:
zero accents is sterile, an accent everywhere is noise, and the Save button is
the one thing on the screen that deserves emphasis.

Every text/background pairing is measured against WCAG AA (4.5:1 normal,
3:1 large), not eyeballed. The values that were nudged to get there are
`MUTED` and `ACCENT_LIT`, with the reason next to each in the source.

## Deliberate non-goals

None of these appear anywhere in the UI, and their absence is a decision:

- **No glass, no glow, no gradients.** Panels separate by three adjacent
  greys. A shadow would make every surface float; elevation is not a
  hierarchy this product needs.
- **No grid, no pattern.** The content pane is a plain scrollable page.
- **No animation.** See MOTION below.
- **No dark/light toggle.** The product impersonates a Discord client; a light
  theme would break that identity. Legitimate under a fixed-theme rule, not an
  excuse to skip requested work.
- **No emoji in UI text.** If a concept needs a mark, it gets a real icon: the
  overlay's crossed-mic and crossed-headphones glyphs are MDI path data with
  the license and viewBox pinned in the source.

## Dials

Be honest, this product is:

- **ENERGY 1** — a dense settings panel and a transparent overlay. No hero, no
  marketing surface, no "how it works" section.
- **RHYTHM 1** — uniform grid. Section blocks at `spacing(24)`, field rows at
  `spacing(8)`, and that is the whole rhythm. Declaring 1 makes the uniformity
  a decision rather than a template artifact.
- **MOTION 1** — hover states only. The speaking ring changes thickness and
  colour and never animates; there are no loops anywhere in the UI. Focusing a
  field off screen jumps the page to it in one step, with no tween.

MOTION 1 is what makes the stillness a choice. If a future pass adds animated
scroll reveal or parallax, it must also raise this dial and say so. An instant
jump is a cut, not motion, so the keyboard focus reveal does not move it.

## Accent's one job

Guide attention to the one actionable element: Save, or the selected section.
Nowhere else. The sidebar's selected item uses `ACCENT_LIT` at 4.70:1 with
white text, which is the same accent one brightness step up.

## Pointers

- `CONTEXT.md` — domain vocabulary and invariants
- `docs/dev/code-layout.md` — module structure
- `docs/adr/` — decisions for significant choices
