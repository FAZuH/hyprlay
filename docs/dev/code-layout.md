# Code layout

A two-crate cargo workspace: one multi-bin package at the root and the
`hyprlay-core` library crate. Every binary links `hyprlay-core`;
dependencies point toward core, never away from it. Core stays free of
UI frameworks and async runtimes so each binary can afford to link it.
The standalone debug probes under `scripts/` are excluded from the
workspace and declare their targets as examples — they never build
with or get installed by the binaries, and keeping them bin-free makes a
bare `cargo install --git` of this repo resolve to exactly one binary
package.

## Package map

| Path | Targets | Role |
|---|---|---|
| `Cargo.toml` + `src/` | 3 bins + doc-hidden lib | Everything user-facing: the `hyprlay`, `hyprlayd`, and `hyprlay-gui` binaries plus their shared code. A bare `cargo install --git <repo>` installs all three |
| `crates/hyprlay-core` | lib | Shared foundation: domain vocabulary (commands, keys, replies), persisted config with its bounds table, framework-free color math, Discord credential storage, compositor discovery, ctl socket protocol |
| `scripts/` | examples only | Standalone debug probes (`wsprobe`, `ipcprobe`) for raw Discord traffic; run via `cargo run -p hyprlay-scripts --example <name>` from that directory |

## Module seams

### hyprlay package

`src/lib.rs` is the doc-hidden library surface: it exposes the three
fronts (`cli`, `daemon`, `gui`) to the thin mains and to integration
tests under `tests/`. The fronts must only meet at `hyprlay-core`
(see Tests layout — `front_isolation.rs` enforces that convention).

| Module | Seam (interface) | What it hides |
|---|---|---|
| `src/bin/hyprlay.rs` | thin main → `cli::run(&args)` | Process entry and exit status only |
| `src/cli/mod.rs` | clap tree + `classify -> Outcome`, `run(args) -> i32` | Argv shape: help/version answered locally, unknown commands and wrong arity rejected as clap-owned (unpinned) text, bad values rejected locally with the daemon's own wording |
| `src/cli/dispatch.rs` | `exec_sibling(name)` | Sibling-binary resolution via `current_exe().parent()`; exec keeps the PID so supervisors keep tracking |
| `src/cli/install.rs` | `install/uninstall(...)` + `Systemctl` trait | Unit and desktop file contents and the exact systemctl call sequence; injectable runner for tests |
| `src/bin/hyprlayd.rs` | thin main → `daemon::run()` | Process entry only |
| `src/daemon/mod.rs` | iced_layershell app shell (`run()`, effect → `Task` translation, subscription wiring, logging init) | Shell-answered commands, single-instance guard, re-exec paths; domain logic lives in the modules below |
| `src/daemon/ctl_server.rs` | `incoming()` stream of `CtlRequest` | Tokio unix-socket listener; the wire vocabulary itself lives in core |
| `src/daemon/overlay/state.rs` | `Overlay` model methods (`desired_size`, `displayed`, `apply_discord`) | Roster filtering, sizing, avatar cache/dedup |
| `src/daemon/overlay/geometry.rs` | `anchor/margin/drag(cfg, …)` | All screen-placement math |
| `src/daemon/overlay/view.rs` | `view(&Overlay)` | Widget construction only |
| `src/daemon/adapters/discord.rs` | `run(sender, auth) -> DiscordEvent` | Local IPC protocol (unix socket), OAuth token exchange, reconnection, voice subscriptions |
| `src/daemon/adapters/ipc.rs` | framed unix-socket client | Discord's local IPC wire format: 8-byte LE header, handshake, socket discovery |
| `src/daemon/adapters/auth.rs` | `detect() -> Option<OwnAppAuth>`, `exchange(code)` | Credential resolution (env → auth.json) and the OAuth code exchange |
| `src/daemon/adapters/{cache,avatar,token}.rs` | roster/avatar/token stores | On-disk persistence with tracing on real failures |
| `src/bin/hyprlay-gui.rs` | thin main → `gui::run()` | Process entry and exit status only |
| `src/gui/mod.rs` | iced app `Gui::run()` | Window layout (header / sidebar / content / status bar), field registry, search; every change becomes a `Command` |
| `src/gui/fields.rs` | per-key field registry | Section, label, tooltip, and control rendering for each setting |
| `src/gui/daemon.rs` | `DaemonState` machine | Status chip states (connecting… / up / daemon not active) and the Start/Stop toggle plumbing (systemctl vs spawn vs `quit`) |
| `src/gui/picker.rs` | color picker widget | Color selection UI |
| `src/gui/theme.rs` | theme | Look and feel constants |

### hyprlay-core

| Module | Seam (interface) | What it hides |
|---|---|---|
| `domain.rs` | `Command::from_str` + `apply_config -> CommandResult`, `Key::ALL` | The whole CLI vocabulary, wire-protocol replies, typed status/colors — pure, no framework types |
| `config.rs` | `load/save/clamp`, `Bounds` | TOML persistence and the single source of truth for every numeric bound (the former `toolkit` `Bounds` dissolved here) |
| `color.rs` | `Rgb`, `Hsv`, conversions | Framework-free HSV/RGB color math (the former toolkit color primitives) |
| `credentials.rs` | `AppCredentials`, auth.json load/save | Discord own-app credential storage; no network IO, never travels the ctl socket |
| `ctl.rs` | socket path, `probe_socket`, `send_command_line`, help/KEYS/FILES/EXAMPLES formatters | Client side of the control protocol plus the shared help text used by both the wire `help` reply and the CLI help |
| `compositor/hyprland.rs` | `Compositor::monitors()` via `detect()` | `hyprctl monitors -j` invocation and JSON parsing |

## Tests layout

Front suites live under the root `tests/` directory — they link the
package's doc-hidden lib through `tests/common/mod.rs`. Core keeps its
own filesystem-touching suites under `crates/hyprlay-core/tests/`.

| Suite | Covers |
|---|---|
| `tests/front_isolation.rs` | Convention enforcement: scans `src/{cli,daemon,gui}` for cross-front imports and fails listing violations — re-arms the front↔front wall that consolidation removed from cargo's hands |
| `tests/dispatch.rs` | sibling resolution and exec-launch behavior |
| `tests/install.rs` | unit/desktop file contents, systemctl call sequences (recorded double), idempotent install/uninstall |
| `tests/token_store.rs` | token cache filesystem behavior |
| `tests/ipc_discover.rs` | Discord IPC socket discovery |
| `crates/hyprlay-core/tests/socket_probe.rs` | `probe_socket` verdicts against real temp-dir sockets |
| `crates/hyprlay-core/tests/credentials.rs` | auth.json save/load roundtrip and rejection of malformed files |

Config roundtrip tests stay inline in core's `config.rs` — they parse
TOML strings in memory and touch no files.

## Notes

- Every numeric knob has one `Bounds` entry in
  `crates/hyprlay-core/src/config.rs`. The parser's error strings
  (`error: opacity <0-100>`), the config-file clamp, the GUI slider
  ranges, and the help text all derive from that table.
- Wire-protocol reply strings (e.g. `opacity=70`, `status=…`) are part
  of the observable contract between daemon and clients; tests pin them.
  CLI-local argv errors are different: clap prints them in its own
  unpinned wording, and locally-printed core errors (bad value, unknown
  key) never travel the socket. Only daemon replies are byte-stable.
- `Command::Reload` performs IO inside `apply_config`; all other commands
  are pure mutations plus an `Effect` list that the daemon shell
  translates into tasks (`Resize`, `Reanchor`, `Nudge`).
