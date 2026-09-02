# ADR-001: Tray runs as a separate resident process

## Status

Accepted

## Date

2026-08-26

## Context

hyprlay needs a system tray (StatusNotifierItem) menu. The user can see
daemon state and control hyprlay without a terminal.

The tray must outlive the daemon. A real Start/Stop daemon toggle is only
possible when the tray survives a "Stop daemon" action.

The repo follows one rule: one process does one thing. Each front is a thin
bin over shared modules. The daemon owns the RPC connection and the overlay.
Stopping it must not stop a control surface.

The tray cannot live inside the daemon. It must run as a fourth resident
process that watches the daemon and sends commands to it.

## Decision

Ship `hyprlay-tray` as a fourth thin bin plus a new front `src/tray`. One
process does one thing.

Use `ksni = "0.3"`. It is a pure-Rust SNI over DBus with no GTK dependency.

Poll daemon status every 2 s. Diff-gate each `handle.update()` call: call it
only on a real state change. Steady state emits zero DBus traffic.

Add single-instance guards with `flock` lock files (`hyprlay-tray.lock`). The
lock self-releases when the process crashes.

Do not add config locking now. The project verified a single writer for every
shared file (config, token, and cache are daemon-only; `auth.json` is
GUI-only). Add locking only when a second writer appears.

## Rejected alternatives

### Tray inside the daemon

A tray living inside the daemon dies when the user stops the daemon. This
breaks the Start/Stop toggle. It also breaks the one-process-one-thing rule.

### GTK-based tray-icon crate

The `tray-icon` crate pulls in GTK. That weight breaks the repo's
minimal-dependency rule (Linux and Hyprland only, no GTK).

### Push/event channel over the ctl socket

A notify-on-change event path over the control socket is not needed now. The
2 s diff-gated poll is simpler and does the job. The wire protocol stays
untouched.

### Tailscale-style "rebuild menu" item

Waybar updates the menu live. The prototype proved this. The rebuild item
only works around buggy host stacks. Revisit it only on a real staleness
report.

## Consequences

Two systemd user units now exist: `hyprlay.service` and
`hyprlay-tray.service` (`Restart=on-failure`). Install verifies every sibling
binary before it writes any unit. It aborts and names the missing binaries.

Amended by ADR-005: the tray is still a separate resident process, but it
is no longer a separate binary — it runs as `hyprlay tray` inside the
launcher.

`tests/front_isolation.rs` now scans `src/tray` for cross-front imports.

The wire protocol does not change. The tray reuses `Command::Status`,
`Set(Key::Visible)`, and `Quit`. No new commands appear.

The tray is a new front. It cannot share code with the GUI because fronts
cannot share code. The start-daemon logic re-implements `plan_action` as a
thin copy.

If no StatusNotifierWatcher host exists, the tray logs the failure once and
keeps running. This state is never fatal.
