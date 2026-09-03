# CONTEXT

Domain vocabulary and invariants for `hyprlay`. Read this before
touching `src/` or `crates/`. ADRs for significant decisions live in
[docs/adr/](docs/adr/).

## What this is

A two-crate Rust workspace. One multi-bin package, `hyprlay`, ships two
binaries (`hyprlay`, `hyprlayd`) as thin `src/bin/` mains. The launcher
routes the shared fronts `src/cli`, `src/gui`, and `src/tray` in-process
through the composition root in `src/lib.rs` (`hyprlay gui`, `hyprlay
tray`); the daemon binary is a thin main over `src/daemon`;
`crates/hyprlay-core` is the separate pure-lib crate. The
package renders a Discord voice-channel roster on a transparent overlay
surface — a Wayland layer-shell surface on Linux, a plain frameless
always-on-top `winit` window on Windows/macOS — plus two control
surfaces (CLI over the platform control transport, settings GUI) that
mutate a shared runtime config. Cross-platform: Linux (Hyprland
first-class; the Linux arm is layer-shell only, so X11 has no overlay
host), Windows, macOS.
Platform mechanics live behind ports in `src/platform/`; see
[docs/adr/004-cross-platform-ports.md](docs/adr/004-cross-platform-ports.md).

## Vocabulary

- **Daemon** — the `hyprlayd` process: owns the Discord RPC connection,
  the overlay window, and the runtime `Config`. Started explicitly
  (`hyprlay daemon`, direct `hyprlayd`) or automatically: opening the
  settings GUI starts it when it is down, or it runs as the platform
  service (systemd user unit / LaunchAgent / Startup item). Bare `hyprlay` prints help and starts nothing. Closing the
  GUI never stops it — daemon lifetime is never tied to a client's.
- **Tray** — the `hyprlay tray` process: a resident system-tray menu
  (StatusNotifierItem over DBus on Linux via `ksni`; `tray-icon`
  NSStatusItem/Shell_NotifyIcon on macOS/Windows). It shows daemon state and sends control commands over the control socket. It outlives the daemon and runs as its own service unit (systemd user unit on Linux, LaunchAgent on macOS, Startup item on Windows). It depends only on `hyprlay-core` plus a lock helper.
- **Client** — any short-lived invocation of the CLI (`hyprlay
  <command>`) or the settings GUI (`hyprlay gui`); both send commands
  to the daemon over **the control socket**
  (`$XDG_RUNTIME_DIR/hyprlay.sock`).
- **Command** — the entire CLI vocabulary, modeled as the `Command` enum in
  `crates/hyprlay-core/src/domain.rs`. Parsing (`FromStr`) and application
  (`apply_config`) are separate steps; semantic parse errors carry the full
  reply text.
- **Reply** — the exact string a command produces (e.g. `opacity=70`,
  `status=…`, `error: opacity <0-100>`). Replies are part of the observable
  contract between daemon and clients; tests pin them byte-for-byte.
- **Effect** — a declarative side effect (`Resize`, `Reanchor`,
  `Nudge`, …) returned alongside a reply. Only the shell
  (`src/daemon`) translates effects into real work (iced tasks); commands stay
  pure.
- **Roster** — the current set of voice-channel participants plus channel
  name and own user id. Persisted to the roster cache only while connected;
  speaking state is never persisted (it would be stale on load).
- **Roster row** — one participant entry rendered on the overlay surface:
  avatar plus username, decorated by speaking ring and mute badges. The
  overlay shows only roster rows — never connection or status text. An
  empty roster renders an empty transparent surface.
- **RosterChange** — `Changed`/`Unchanged` result of applying a Discord
  event to the `Overlay`; drives cache writes and view refreshes.
- **Overlay layer** — the Wayland layer-shell layer the overlay binds to.
  `Top` (normal) is below fullscreen windows; `Overlay` is above them.
  Controlled by `show-on-fullscreen` and requires a daemon restart to
  re-bind (same path as `monitor`).
- **Dim on hover** — when `dim-on-hover` is on, a pointer inside the
  overlay's geometry switches the rendered overall opacity from `opacity`
  to `hover-opacity`. Detection keeps `events_transparent:true` (full
  click-through) and polls the platform's global cursor position —
  Hyprland's IPC socket on Hyprland, an OS query (`GetCursorPos`,
  `NSEvent.mouseLocation`, `XQueryPointer`) on Windows/macOS/X11 —
  socket vs the overlay rect; it is a no-op when `visible` is off or the
  roster is empty. Requires a global cursor source: on Hyprland via its
  socket, Windows/macOS/X11 via OS queries; a no-op on non-Hyprland
  Wayland.
- **Hover opacity** — target overall opacity (0..100, default 40) while
  hovered. It replaces `opacity` in `Alphas` during hover and multiplies
  into avatar/text/box.
- **Bounds** — the inclusive min/max range of one numeric knob, declared
  once in `crates/hyprlay-core/src/config.rs`. Parser validation, config
  clamping, GUI slider ranges, error text, and help text all derive from it.

## Invariants

- The wire protocol between clients and the daemon is byte-stable. Changing
  a reply string or error format is a breaking change requiring test updates.
  New commands may be appended without breaking it (`quit` was added this way).
- CLI-local argv parse errors (unknown command, wrong arity, help/version)
  are clap-owned text printed by the `hyprlay` bin. They are NOT part of the
  byte-stable wire contract; only daemon replies keep that guarantee.
- No client secret ships in the binary. Sign-in uses only the user's own
  Discord application credentials (client id + secret via environment
  variables, `auth.json`, or the GUI). There is no built-in fallback
  identity. See [docs/token-exchange.md](docs/token-exchange.md).
- Tokens and OAuth codes are never written to logs.
- Every numeric config value is clamped on load; hand-edited configs can
  never produce a broken overlay.
- Dependency direction: the four fronts (`cli`, `daemon`, `gui`, `tray`) are
  modules of one package; each depends only on `hyprlay-core`, and core
  never imports UI-framework or async-runtime types. The one sanctioned
  cross-front meeting point is the composition root (`run` in
  `src/lib.rs`), which routes `gui`/`tray` in-process. Inside the daemon
  front: core ← adapters ← overlay/ctl ← shell. Front↔front separation
  is a convention, not a compiler wall: consolidation removed cargo's
  per-crate boundary, so `tests/front_isolation.rs` re-arms it by
  scanning `src/{cli,daemon,gui,tray}` for cross-front imports on every
  `cargo test` (the composition root sits outside the scanned front
  dirs). The CLI adds only clap; the GUI adds iced; the tray adds
  ksni (Linux) or tray-icon (Windows/macOS); adapters never import UI
  modules. Platform adapters live in `src/platform/` (not a front) and
  are the only place platform crates are imported; fronts reach them
  through `hyprlay-core` ports and the platform module's factories.
- Daemon-side commands (`save`, `dump`, `status`, `help`, `get`,
  `restart`, `quit`, `set monitor`, `set show-on-fullscreen`) are
  answered by the shell before reaching `apply_config` (they re-bind the
  layer surface and need a restart).

## Glossary pointers

- Module seams table: [docs/dev/code-layout.md](dev/code-layout.md)
- Logging design (stderr vs JSON wide events): README "Logs"
