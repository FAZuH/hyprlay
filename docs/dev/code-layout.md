# Code layout

A two-crate cargo workspace: one multi-bin package at the root and the
`hyprlay-core` library crate. Every binary depends on `hyprlay-core`
(Dependency Inversion). Dependencies point toward the stable core,
never away from it. Core has high cohesion and no UI or async
frameworks, so each binary can link it with low coupling. The standalone debug probes under `scripts/` are excluded from the
workspace and declare their targets as examples. They never build
with or get installed by the binaries. Keeping them bin-free makes
a bare `cargo install --git` of this repo resolve to exactly one
binary package.

## Package map

| Path | Targets | Role |
|---|---|---|
| `Cargo.toml` + `src/` | 2 bins + doc-hidden lib | Everything user-facing: the `hyprlay` and `hyprlayd` binaries plus their shared code. The launcher runs its `gui`/`tray` fronts in-process (`hyprlay gui`, `hyprlay tray`) through the composition root in `src/lib.rs`. A bare `cargo install --git <repo>` installs both |
| `crates/hyprlay-core` | lib | Shared foundation: domain vocabulary (commands, keys, replies), persisted config with its bounds table (single source of truth), framework-free color math, Discord credential storage, compositor/cursor port traits, the `Platform` facade, ctl socket protocol |
| `scripts/` | examples only | Standalone debug probes (`wsprobe`, `ipcprobe`) for raw Discord traffic; run via `cargo run -p hyprlay-scripts --example <name>` from that directory |

## Module interfaces

Each module has a single responsibility (SRP) and hides its
implementation behind a narrow public interface (Encapsulation).
High-level policy depends on abstractions in `hyprlay-core`
(Dependency Inversion). The interfaces below enforce high cohesion
within each module and low coupling between modules. Fronts depend only
on the core abstractions plus the `src/platform/` adapter factories —
never on a platform crate (Interface Segregation).

### hyprlay package

`src/lib.rs` is the doc-hidden library surface: it exposes the four
fronts (`cli`, `daemon`, `gui`, `tray`) to the thin mains and to
integration tests under `tests/`, and hosts the composition root
`run(args)` that routes `hyprlay gui` / `hyprlay tray` in-process. The
fronts achieve low coupling by depending only on the shared abstraction
`hyprlay-core`, never on each other; the fronts meet at `hyprlay-core`
and at that composition root, which sits outside the scanned front
directories. `tests/front_isolation.rs` enforces that encapsulation
boundary.

| Module | Public interface | Encapsulated responsibility |
|---|---|---|
| `src/bin/hyprlay.rs` | thin main → `run(&args)` in `src/lib.rs` | Process entry and exit status only; the composition root in `lib.rs` classifies argv and routes `gui`/`tray` in-process, everything else to `cli::execute` |
| `src/cli/mod.rs` | clap tree + `classify -> Outcome`, `execute(outcome) -> i32` | Argv shape: help/version answered locally, unknown commands and wrong arity rejected as clap-owned (unpinned) text, bad values rejected locally with the daemon's own wording |
| `src/cli/dispatch.rs` | `exec_sibling(name)` | Sibling-binary resolution via `current_exe().parent()`; exec keeps the PID so supervisors keep tracking |
| `src/cli/install.rs` | `run_install`/`run_uninstall` | Thin resolver: real config/data/exe dirs in, the platform's install/uninstall flow out (`src/platform/service/`), report printed; the unit/registry writing lives in the platform adapters |
| `src/bin/hyprlayd.rs` | thin main → `daemon::run()` | Process entry only |
| `src/daemon/mod.rs` | daemon shell (`run()`, effect → `Task` translation, subscription wiring, logging init) | Shell-answered commands, single-instance guard, re-exec paths, command resolution shared by both surface hosts; domain logic lives in the modules below |
| `src/daemon/surface_host/mod.rs` | `run(cfg, auth) -> ExitCode` | `#[cfg]` dispatch between the two overlay shells; roster state and domain logic stay in the parent `daemon` module |
| `src/daemon/surface_host/layershell.rs` | Linux/Wayland overlay shell | The existing `iced_layershell` app, behaviour byte-identical: edge anchoring with margins, hover polling |
| `src/daemon/surface_host/winit.rs` | Windows/macOS overlay shell | Frameless, transparent, always-on-top `iced` window moved to the computed on-screen position; same shared logic and hover poll |
| `src/daemon/ctl_server.rs` | `incoming()` stream of `CtlRequest` | Serves the core `ControlListener` on a dedicated thread (accept loop never stalls the async host), one thread per connection; the wire vocabulary itself lives in core (single source of truth) |
| `src/daemon/overlay/state.rs` | `Overlay` model methods (`desired_size`, `displayed`, `apply_discord`) | Roster filtering, sizing, avatar cache/dedup |
| `src/daemon/overlay/geometry.rs` | `anchor/margin/drag(cfg, …)` | All screen-placement math |
| `src/daemon/overlay/view.rs` | `view(&Overlay)` | Widget construction only |
| `src/daemon/adapters/discord.rs` | `run(sender, auth) -> DiscordEvent` | Local IPC protocol over `IpcStream`, OAuth token exchange, reconnection, voice subscriptions (Adapter to external Discord API) |
| `src/daemon/adapters/ipc.rs` | transport-agnostic `IpcStream` + `DiscordTransport` port | Discord's local IPC wire format: 8-byte LE header, handshake, PING/PONG; per-OS discovery + connect (unix socket / named pipe) behind the package-local `DiscordTransport` port (Adapter) |
| `src/daemon/adapters/auth.rs` | `detect() -> Option<OwnAppAuth>`, `exchange(code)` | Credential resolution (env → auth.json) and the OAuth code exchange |
| `src/daemon/adapters/{cache,avatar,token}.rs` | roster/avatar/token stores | On-disk persistence with tracing on real failures |
| `src/gui/mod.rs` | iced app shell: `Gui::run()` | `Message`, `Gui`, boot/subscribe wiring, window settings, and the blocking `send` wrapper; every change stays a `Command` — the layer modules below own the rest |
| `src/gui/update.rs` | `pub(super)` `update(gui, msg)` | The one flat update match (the app's dispatch table), the `shortcut` dispatcher, and the async daemon-toggle / auth effects |
| `src/gui/commands.rs` | `pub(super)` `command_for`, `apply_num`, `revert_commands` | Message → Command translation plus the bookkeeping the update arms share: unsaved marker, numeric bounds check, revert diff |
| `src/gui/scroll.rs` | `pub(super)` `measure_sections`, `scroll_to_section` | One-page navigation: the measure operation, section jumps, scrollspy highlight, and the shared widget ids |
| `src/gui/view.rs` | `pub(super)` `view(gui)` | Window composition: header (title, search, global actions), sidebar (section anchors), status bar (unsaved marker, daemon toggle, last reply) |
| `src/gui/fields.rs` | per-key field registry | Section, label, tooltip, and control rendering for each setting |
| `src/gui/daemon.rs` | `DaemonState` machine | Status chip states (connecting… / up / daemon not active) and the Start/Stop toggle plumbing (systemctl vs spawn vs `quit`) |
| `src/gui/picker.rs` | color picker widget | Color selection UI |
| `src/gui/theme.rs` | theme | Look and feel constants |
| `src/tray/mod.rs` | `run()` + shared poll loop | Tray icon lifecycle: 2 s diff-gated status poll, action handling, platform backend dispatch (`ksni` / `tray-icon`) behind the `Tray` port |
| `src/tray/port.rs` | `Tray` trait (`update`, `shutdown`) | The tray backend port — package-local so the `image` codec behind `IconData` stays out of the framework-free core |
| `src/tray/menu.rs` | menu builder | Menu structure and action routing |
| `src/tray/icon.rs` | icon resolver | Icon selection and loading |
| `src/tray/daemon.rs` | `DaemonState` bridge | Daemon status observation for tray |
| `src/platform/` | `detect()` factories, `SystemControl`, `Control`, `host()` | The only adapter layer: every platform crate import lives here, `#[cfg]`-gated per target; fronts call these factories instead of naming an OS |
| `src/platform/compositor/` | `detect() -> Box<dyn Compositor>` | Compositor adapter selection per session (Hyprland today, `Unknown` no-op otherwise) |
| `src/platform/cursor/` | `detect()`, `cursor_pos()` | Global-cursor adapters: Hyprland socket fast path, X11, Win32, macOS; `NoCursor` where no portable read exists |
| `src/platform/ipc/` | `Control` (`ControlEndpoint` + `ControlListener`) | Unix-socket / named-pipe transport for the ctl channel |
| `src/platform/service/` | `SystemControl`, `install_service`/`uninstall_service` | systemd / launchd / Windows backends behind the core `ServiceManager` port; owns the unit/registry writing and the exact systemctl calls |
| `src/platform/tray/` | `ksni::run`, `tray_icon::run` | Tray backends behind the package-local `Tray` port |
| `src/platform/host.rs` | `host()` | The core `Platform` port: detached child spawn per OS |

### hyprlay-core

Core is the stable abstraction that all fronts depend on (Dependency
Inversion, Stable-dependencies principle). Each module has high cohesion
and a single reason to change.

| Module | Public interface | Encapsulated responsibility |
|---|---|---|
| `domain.rs` | `Command::from_str` + `apply_config -> CommandResult`, `Key::ALL` | The whole CLI vocabulary, wire-protocol replies, typed status/colors — pure, no framework types; single source of truth for commands |
| `status.rs` | `StatusFields` + `to_wire`/`parse_wire`/`is_status_line` | The `status` reply wire contract in both directions — build and parse live together so field order and spelling change in one place |
| `config.rs` | `load/save/clamp`, `Bounds` | TOML persistence and the single source of truth for every numeric bound (the former `toolkit` `Bounds` dissolved here) |
| `color.rs` | `Rgb`, `Hsv`, conversions | Framework-free HSV/RGB color math (the former toolkit color primitives) |
| `credentials.rs` | `AppCredentials`, auth.json load/save | Discord own-app credential storage; no network IO, never travels the ctl socket |
| `ctl.rs` | `ControlStream`/`ControlEndpoint`/`ControlListener` transport ports, socket path, `probe_socket`, `send_command_line`, help/KEYS/FILES/EXAMPLES formatters | Both ends of the control protocol as transport-agnostic ports plus the pure framing/decision logic and the shared help text; the unix-socket / named-pipe adapters live in the host package's `src/platform/` |
| `compositor/` | `Compositor` + `CursorSource` traits, `parse_cursor_reply`, cursor coordinate converters | Ports only: monitor and global-cursor reads behind two traits plus pure parsing/coordinate math; the concrete adapters live in the host package's `src/platform/` |
| `platform.rs` | `Platform` trait, `runtime_dir`/`state_dir`/`secure_perms` | The OS facts the pure core needs, as pure helpers; detached spawn stays behind the `Platform` port, implemented by the host package's `src/platform/host.rs` |

## Tests layout

Front suites live under the root `tests/` directory — they link the
package's doc-hidden lib through `tests/common/mod.rs`. Core keeps its
own filesystem-touching suites under `crates/hyprlay-core/tests/`.

| Suite | Covers |
|---|---|
| `tests/front_isolation.rs` | Encapsulation-boundary enforcement: scans `src/{cli,daemon,gui,tray}` for cross-front imports and fails on violations — enforces low coupling and the acyclic dependencies principle |
| `tests/dispatch.rs` | sibling resolution and exec-launch behavior |
| `tests/install.rs` | unit/desktop file contents, systemctl call sequences (recorded test double), idempotent install/uninstall |
| `tests/token_store.rs` | token cache filesystem behavior |
| `tests/ipc_discover.rs` | Discord IPC socket discovery |
| `crates/hyprlay-core/tests/socket_probe.rs` | `probe_socket` verdicts against real temp-dir sockets |
| `crates/hyprlay-core/tests/credentials.rs` | auth.json save/load roundtrip and rejection of malformed files |

Config roundtrip tests stay inline in core's `config.rs` — they parse
TOML strings in memory and touch no files. They test through the
public API.

## Notes

- Every numeric knob has one `Bounds` entry in
  `crates/hyprlay-core/src/config.rs` (single source of truth). The
  parser's error strings (`error: opacity <0-100>`), the config-file
  clamp, the GUI slider ranges, and the help text all derive from that
  table.
- Wire-protocol reply strings (e.g. `opacity=70`, `status=…`) are part
  of the observable contract between daemon and clients; tests pin them.
  CLI-local argv errors are different: clap prints them in its own
  unpinned wording, and locally-printed core errors (bad value, unknown
  key) never travel the socket. Only daemon replies are byte-stable.
- `Command::Reload` performs IO inside `apply_config`. All other commands
  are pure mutations plus an `Effect` list that the daemon shell
  translates into tasks (`Resize`, `Reanchor`, `Nudge`). This separates
  pure domain logic from side effects (Command–Query Separation,
  high cohesion).
