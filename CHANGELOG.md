# Changelog

## [Unreleased]

- Add tray menu

### Overlay
- Added show on fullscreen to keep roster above fullscreen windows
- Added dim on hover to lower opacity while pointer is over overlay
- Added hover opacity setting for dimmed level

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
