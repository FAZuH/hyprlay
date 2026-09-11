# ADR-002: Overlay stays above fullscreen via Layer::Overlay

## Status
Accepted

## Date
2026-08-27

## Context
The overlay is a `Layer::Top` surface with `events_transparent:true` (`src/daemon/mod.rs:126-133`). On Hyprland any fullscreen window (any `fullscreen != 0`) renders above `Top`, so the roster disappears — the same reason a `Top` panel hides behind a fullscreen game. Windows Discord keeps its overlay above fullscreen by default, and the user requested that parity with a toggle to turn it off.

Two ways to stay above fullscreen on Hyprland exist:

1. **Layer-shell `Layer::Overlay`** — portable Wayland, guaranteed above fullscreen by spec. Requires re-creating the layer surface (the output/monitor binding is fixed at surface creation, same as `monitor`).
2. **Hyprland `layerrule`** (`layerrule = ...` / `hyprctl keyword layerrule`) — Hyprland-only, mutates compositor config at runtime, avoids restart.

The tray feature already established that `monitor` changes re-create the surface via `exec`-restart (`src/daemon/mod.rs:426-433`, `can_reexec`, `restart_daemon`) because layer-shell surfaces bind to an output at creation.

Hover-dim must preserve click-through, so `events_transparent:true` stays. iced `mouse_area` hover is not used; cursor position is polled over the Hyprland IPC socket vs the overlay rect.

## Decision
- Introduce `show-on-fullscreen` (bool, default `on`, `[layout]`), `dim-on-hover` (bool, default `off`, `[layout]`), `hover-opacity` (u8 0..100 default 40, `[opacity]`). All are `Key::` variants, `get`/`set`/wire-stable, `auto_save`-persisted. `reset layout` covers the two flags, `reset opacity` covers `hover-opacity`.
- `show-on-fullscreen` maps to `Layer::Overlay` (on) vs `Layer::Top` (off) at daemon startup `src/daemon/mod.rs:124-126`. Toggling it is daemon-side like `set monitor`: validate, `config.save()`, guard `can_reexec()`, reply, then `restart_daemon()`. The layer switch is **universal** (not Hyprland-conditional) and counts **any fullscreen** (`!= 0`), consistent with Windows.
- Hover keeps `events_transparent:true`. When `dim-on-hover=on && visible && !displayed().is_empty()`, poll Hyprland cursor position every 50 ms (20 Hz) over `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock` (`cursorpos` request), hit-test against `geometry::overlay_rect` (anchor/offset/size + monitor x/y from `compositor::Hyprland`), and flip `Overlay.hovered`. View renders `alphas_for(hovered)` where `overall` is `hover-opacity` when hovered. No polling when disabled/empty/off — zero cost. Non-Hyprland (`Unknown` compositor) never hovers.

## Rejected alternatives

### layerrule / hyprctl keyword
Hyprland-only and mutates external compositor state. It avoids restart but adds IPC that writes Hyprland config, is fragile across Hyprland versions, and doesn't help on non-Hyprland sessions. `Layer::Overlay` is portable, spec-guaranteed, and reuses the existing restart path users already understand for `monitor`. A layerrule optimization can be added later without changing the config surface.

### iced mouse_area with events_transparent:false
Giving the surface pointer input lets iced detect hover natively, but the overlay then **blocks clicks** to the window behind it. That defeats the stated goal ("see where I want to click without moving the roster"). Input-region shaping to make gaps pass through is more complex and still Hyprland-specific for correctness.

### Single knob where hover-opacity=100 means disabled
Conflates "whether" with "how much" and makes the GUI slider ambiguous. Two knobs (`dim-on-hover` + `hover-opacity`) mirror the existing `visible` + `opacity` pattern and let the slider be greyed when disabled.

## Consequences
- Toggling `show-on-fullscreen` costs a daemon restart (control socket downtime ~100 ms, roster re-hydrated from cache). Same cost/UX as `monitor` changes.
- Hover polling is 20 Hz only while enabled and displayable; otherwise no cursor IPC traffic. View re-renders only on hover edge transitions (diff-gated).
- `status` reply appends `show-on-fullscreen`, `dim-on-hover`, `hover-opacity` at the end. Amended 2026-09-03: the convention is now structural — `hyprlay_core::status::StatusFields` builds and parses the whole line, so appending a field is a one-place edit and readers survive it by construction (the hand-rolled token parsers this bullet originally justified are gone).
- `CONTEXT.md` gains `Overlay layer`, `Dim on hover`, `Hover opacity` vocabulary; the `Daemon-side commands` invariant now lists `set show-on-fullscreen`.
