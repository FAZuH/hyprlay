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
| `Cargo.toml` + `src/` | 4 bins + doc-hidden lib | Everything user-facing: the `hyprlay`, `hyprlayd`, `hyprlay-gui`, and `hyprlay-tray` binaries plus their shared code. A bare `cargo install --git <repo>` installs all four |
| `crates/hyprlay-core` | lib | Shared foundation: domain vocabulary (commands, keys, replies), persisted config with its bounds table (single source of truth), framework-free color math, Discord credential storage, compositor discovery, ctl socket protocol |
| `scripts/` | examples only | Standalone debug probes (`wsprobe`, `ipcprobe`) for raw Discord traffic; run via `cargo run -p hyprlay-scripts --example <name>` from that directory |

## Module interfaces

Each module has a single responsibility (SRP) and hides its
implementation behind a narrow public interface (Encapsulation).
High-level policy depends on abstractions in `hyprlay-core`
(Dependency Inversion). The interfaces below enforce high cohesion
within each module and low coupling between modules. Fronts depend only
on the core abstraction (Interface Segregation).

### hyprlay package

`src/lib.rs` is the doc-hidden library surface: it exposes the four
fronts (`cli`, `daemon`, `gui`, `tray`) to the thin mains and to
integration tests under `tests/`. The fronts achieve low coupling by
depending only on the shared abstraction `hyprlay-core`, never on each
other. `tests/front_isolation.rs` enforces that encapsulation boundary.

| Module | Public interface | Encapsulated responsibility |
|---|---|---|
| `src/bin/hyprlay.rs` | thin main → `cli::run(&args)` | Process entry and exit status only |
| `src/cli/mod.rs` | clap tree + `classify -> Outcome`, `run(args) -> i32` | Argv shape: help/version answered locally, unknown commands and wrong arity rejected as clap-owned (unpinned) text, bad values rejected locally with the daemon's own wording |
| `src/cli/dispatch.rs` | `exec_sibling(name)` | Sibling-binary resolution via `current_exe().parent()`; exec keeps the PID so supervisors keep tracking |
| `src/cli/install.rs` | `install/uninstall(...)` + `Systemctl` trait | Unit and desktop file contents and the exact systemctl call sequence; injectable collaborator for tests (Dependency Injection) |
| `src/bin/hyprlayd.rs` | thin main → `daemon::run()` | Process entry only |
| `src/daemon/mod.rs` | iced_layershell app shell (`run()`, effect → `Task` translation, subscription wiring, logging init) | Shell-answered commands, single-instance guard, re-exec paths; domain logic lives in the modules below |
| `src/daemon/ctl_server.rs` | `incoming()` stream of `CtlRequest` | Tokio unix-socket listener; the wire vocabulary itself lives in core (single source of truth) |
| `src/daemon/overlay/state.rs` | `Overlay` model methods (`desired_size`, `displayed`, `apply_discord`) | Roster filtering, sizing, avatar cache/dedup |
| `src/daemon/overlay/geometry.rs` | `anchor/margin/drag(cfg, …)` | All screen-placement math |
| `src/daemon/overlay/view.rs` | `view(&Overlay)` | Widget construction only |
| `src/daemon/adapters/discord.rs` | `run(sender, auth) -> DiscordEvent` | Local IPC protocol (unix socket), OAuth token exchange, reconnection, voice subscriptions (Adapter to external Discord API) |
| `src/daemon/adapters/ipc.rs` | framed unix-socket client | Discord's local IPC wire format: 8-byte LE header, handshake, socket discovery (Adapter) |
| `src/daemon/adapters/auth.rs` | `detect() -> Option<OwnAppAuth>`, `exchange(code)` | Credential resolution (env → auth.json) and the OAuth code exchange |
| `src/daemon/adapters/{cache,avatar,token}.rs` | roster/avatar/token stores | On-disk persistence with tracing on real failures |
| `src/bin/hyprlay-gui.rs` | thin main → `gui::run()` | Process entry and exit status only |
| `src/bin/hyprlay-tray.rs` | thin main → `tray::run()` | Process entry and exit status only |
| `src/gui/mod.rs` | iced app `Gui::run()` | Window layout (header / sidebar / content / status bar), field registry, search; every change becomes a `Command` |
| `src/gui/fields.rs` | per-key field registry | Section, label, tooltip, and control rendering for each setting |
| `src/gui/daemon.rs` | `DaemonState` machine | Status chip states (connecting… / up / daemon not active) and the Start/Stop toggle plumbing (systemctl vs spawn vs `quit`) |
| `src/gui/picker.rs` | color picker widget | Color selection UI |
| `src/gui/theme.rs` | theme | Look and feel constants |
| `src/tray/mod.rs` | `Tray::run()` via `ksni` | Tray icon lifecycle and D-Bus registration |
| `src/tray/menu.rs` | menu builder | Menu structure and action routing |
| `src/tray/icon.rs` | icon resolver | Icon selection and loading |
| `src/tray/daemon.rs` | `DaemonState` bridge | Daemon status observation for tray |

### hyprlay-core

Core is the stable abstraction that all fronts depend on (Dependency
Inversion, Stable-dependencies principle). Each module has high cohesion
and a single reason to change.

| Module | Public interface | Encapsulated responsibility |
|---|---|---|
| `domain.rs` | `Command::from_str` + `apply_config -> CommandResult`, `Key::ALL` | The whole CLI vocabulary, wire-protocol replies, typed status/colors — pure, no framework types; single source of truth for commands |
| `config.rs` | `load/save/clamp`, `Bounds` | TOML persistence and the single source of truth for every numeric bound (the former `toolkit` `Bounds` dissolved here) |
| `color.rs` | `Rgb`, `Hsv`, conversions | Framework-free HSV/RGB color math (the former toolkit color primitives) |
| `credentials.rs` | `AppCredentials`, auth.json load/save | Discord own-app credential storage; no network IO, never travels the ctl socket |
| `ctl.rs` | socket path, `probe_socket`, `send_command_line`, help/KEYS/FILES/EXAMPLES formatters | Client side of the control protocol plus the shared help text used by both the wire `help` reply and the CLI help |
| `compositor/hyprland.rs` | `Compositor::monitors()` via `detect()` | `hyprctl monitors -j` invocation and JSON parsing (Adapter to compositor) |

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
