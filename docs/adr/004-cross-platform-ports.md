# ADR-004: Cross-platform ports: platform mechanics behind core-owned traits

## Status

Accepted

## Date

2026-09-02

## Context

hyprlay is Linux/Hyprland-only today. Every unix-only import is
unconditional. There is no `cfg` guard anywhere. The workspace-level iced
`wayland` feature plus `iced_layershell` gate the whole build on Wayland.

The platform-locked mechanics:

- The control socket is a unix domain socket (`$XDG_RUNTIME_DIR/hyprlay.sock`).
- Discord IPC is a unix socket.
- The tray is ksni/SNI over DBus.
- Single-instance is `flock` lock files.
- Services are systemd user units.
- The overlay is an `iced_layershell` layer-shell surface.
- Hover polling reads the Hyprland cursor over its IPC socket (ADR-002).

We are porting to Windows, macOS, and other Linux compositors (X11 and
non-Hyprland Wayland). The port touches every mechanic above. The question
is where the platform code lives.

## Decision

Ports and adapters, hexagonal style. `hyprlay-core` stays pure: it owns the
domain (`Command`, `Reply`, `Effect`, `Bounds`) and the core port traits
(two ports ended up package-local — see Amendment). Adapters live in
`src/platform/`, a module of the `hyprlay` package, `cfg`-gated per target
and selected there by `detect()`/factory functions. `src/platform/` is the
composition root: the four fronts (`cli`, `daemon`, `gui`, `tray`) call it
directly (e.g. `crate::platform::cursor::detect()`,
`crate::platform::service::SystemControl`) and target dispatch sits at the
composition points (`src/daemon/surface_host/mod.rs`, `src/tray/mod.rs`).
Dependency inversion: a front depends only on core and the `src/platform/`
seam — never on a platform crate.

1. **Ports.** `Compositor` (exists), `CursorSource`, `ControlEndpoint`
   (the control socket transport), `DiscordTransport` (framed JSON codec
   unchanged; unix sockets on Linux/macOS, named pipe
   `\\.\pipe\discord-ipc-N` on Windows), `Tray`, `ServiceManager`, and a
   `Platform` facade (runtime/state dir resolution, spawn, secure file
   permissions). `DiscordTransport` and `Tray` ended up package-local
   rather than core-owned (Amendment below).
2. **Overlay surface.** A `SurfaceHost` abstraction with two arms.
   `iced_layershell` on Linux/Wayland keeps current behaviour
   byte-identical. Plain `iced` + `winit` covers Windows/macOS/X11:
   frameless, transparent, always-on-top, click-through through the boolean
   cursor hit-test, anchored with `Position::Specific` from monitor
   enumeration.
3. **Hover stays Option A.** The overlay stays fully click-through on every
   platform. Dim-on-hover polls the global cursor and checks overlap with
   the overlay rect: `GetCursorPos` on Windows, `NSEvent.mouseLocation` on
   macOS, `XQueryPointer` on X11. Each source is a `CursorSource` adapter.
   Hyprland keeps its socket fast path. Non-Hyprland Wayland has no
   standard global-cursor read (upstream rejected it, wayland#383), so
   hover degrades to a no-op there.
4. **Local IPC.** `interprocess` 2.x behind `ControlEndpoint` and
   `DiscordTransport`: one API for unix sockets and named pipes, with tokio
   support. macOS has no `XDG_RUNTIME_DIR`, so resolution falls back along
   a chain to `TMPDIR`. Each candidate is probed for existence instead of
   trusting the first defined variable. Paths stay under the 104-byte
   `sun_path` cap.
5. **Singleton.** `fd-lock` (flock on unix, `LockFile` on Windows) behind
   the existing `Singleton`/`AcquireError` API.
6. **Tray.** `ksni` stays on Linux: tokio-native, no GTK link deps, and it
   supports icon-click. `tray-icon` covers Windows (`Shell_NotifyIcon`) and
   macOS (`NSStatusItem`, main thread). One shared menu model.
7. **Services.** The `ServiceManager` port: systemd user units on Linux
   (current), launchd LaunchAgent on macOS, Windows startup (Run key or
   Startup folder).
8. **Build gating.** iced's `wayland` feature and `iced_layershell` become
   target-specific dependencies. Plain `iced` (winit) builds everywhere;
   the GUI front already does.
9. **Release CI.** Only local no-underscore workflows change: `build.yml`
   gains a 4-target × 4-binary matrix; `release.yml` gets
   `platforms: multi`. Underscore-prefixed workflows are synced
   byte-identical from the project-ops repo and are never edited here.
10. **Secure file permissions.** `chmod 0600` on unix. On Windows, rely on
    the user-profile ACL; there are no unix mode bits. This difference is
    accepted.

ADR-001 keeps its force on every platform. The Tray stays a separate
resident process. Only its backend — ksni versus tray-icon — is
adapter-swapped.

## Rejected alternatives

### Surface-local hover (Option B)

Give the overlay pointer input and let iced/winit report hover natively.
Rejected. It conflicts with full click-through: accepting pointer input
blocks clicks to the window behind the overlay — the same failure ADR-002
rejected. And winit/iced expose only a boolean cursor hit-test; input
shaping per region needs a native shim per OS.

### One tray crate everywhere

`tray-icon` cannot do icon-click on Linux (a libappindicator limitation),
and ksni speaks only SNI over DBus, so it cannot serve Windows or macOS.
Two backends behind one port is the only combination that keeps icon-click
on Linux.

### `cfg` gates inside the fronts

Platform code could sit inline in each front at every call site behind
`cfg` guards. Rejected: every front would carry platform branches through
its logic, and the adapter seam would decay. What is enforced instead: no
platform-crate imports (`ksni`, `tray-icon`, `iced_layershell`,
`windows-sys`, …) outside `src/platform/`, and target dispatch stays at
the composition points (`src/daemon/surface_host/mod.rs`,
`src/tray/mod.rs`) instead of scattering through the front logic.

## Consequences

- GNOME on Wayland shows no overlay at all: no layer-shell protocol
  support, and a toplevel cannot position itself. Documented as
  unsupported.
- GNOME needs the AppIndicator extension for *any* SNI tray: without it the
  tray icon is hidden regardless of backend. Linux keeps `ksni` rather than
  `tray-icon` for this reason too, since `tray-icon`'s Linux backend cannot
  deliver icon-click events (a libappindicator limitation).
- Non-Hyprland Wayland has no global-cursor read, so dim-on-hover no-ops
  there. KWin and wlroots escape hatches are possible future adapters, not
  in scope.
- macOS: winit does not expose true non-activating `NSPanel` behaviour
  (upstream winit#4670). If needed later, it requires objc2-app-kit on the
  raw handle. winit monitor-origin coordinates also mix physical and
  logical units on multi-display retina setups (upstream winit#2645);
  coordinate conversion must stay per-platform.
- `iced_layershell` is deprecated upstream (superseded by
  `iced_exwlshell` 0.20+). We stay on 0.19.1 for now. Migration is a
  follow-up, not part of this decision.
- Core purity is enforced by CI target checks. New platform imports must
  land in `src/platform/` behind `cfg` gates — never in core, never in the
  fronts directly.
- Replies keep their byte-stable contract on every platform; only the
  transport under `ControlEndpoint` changes. The Discord framed JSON codec
  is likewise unchanged.

## Amendment

2026-09-02, after implementation review:

- Constructor injection into the fronts was not realized. `src/platform/`
  is the composition root: fronts call its `detect()`/factory functions
  directly, and the surface-host/tray target dispatch is
  `#[cfg(target_os)]` at the composition points. The invariant that
  matters still holds: `src/platform/` is the only adapter layer, and
  fronts never import platform crates (`ksni`, `tray-icon`,
  `iced_layershell`, `windows-sys`, …) directly.
- Core does not own every port. `Tray` lives in `src/tray/port.rs`: its
  state types carry `IconData`, decoded from an embedded PNG by the
  `image` crate, and the framework-free core must not take on a codec
  dependency. `DiscordTransport` lives in `src/daemon/adapters/ipc.rs`,
  for symmetry with its tokio consumer (the async daemon protocol code;
  core stays sync and runtime-free). Both keep the hexagonal shape — a
  port beside the adapters and consumer that serve it.
- X11 has no overlay host. The `SurfaceHost` dispatch is
  `#[cfg(target_os)]`, so every Linux target — X11 included — takes the
  layer-shell arm, and the winit arm compiles only off-Linux. X11 keeps
  only the `XQueryPointer` cursor adapter (dormant until an X11 host
  exists). Runtime session-type dispatch (Wayland versus X11) is a
  possible follow-up, not part of this port.
