# Changelog

## [Unreleased]

### GUI

- Show all settings on one scrollable page

## 0.2.0 (2026-08-30)

- Add tray menu

### Overlay
- Added "show over fullscreen" config to keep roster above fullscreen windows
- Added "dim on hover" config to dim while cursor is over overlay
- Added hover opacity setting

## 0.1.0 (2026-08-23)

A lightweight Discord voice overlay for Hyprland.

- Always-on-top voice overlay pinned to any screen edge
- Click-through rendering; hold left button to drag
- Placement on any monitor with presets, anchors, offsets, and nudges
- Speaking ring, custom colors, opacity, width, and text-size controls
- Talking-only mode
- Live settings through get and set, clamped to valid ranges
- Short help with examples for every command
- Settings window controlling daemon and overlay live
- Sign-in through your own Discord application
- Token cache with owner-only permissions
- systemd user service and desktop menu entry
- Idempotent install and uninstall
- TOML config with auto-save
