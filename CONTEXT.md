# CONTEXT

Domain vocabulary and invariants for `hyprlay`. Read this before
touching `src/` or `crates/`. ADRs for significant decisions live in
[docs/adr/](docs/adr/).

## What this is

A two-crate Rust workspace. One multi-bin package, `hyprlay`, ships all
three binaries (`hyprlay`, `hyprlayd`, `hyprlay-gui`) as thin
`src/bin/` mains over the shared modules `src/cli`, `src/daemon`, and
`src/gui`; `crates/hyprlay-core` is the separate pure-lib crate. The
package renders a Discord voice-channel roster on a transparent Wayland
layer-shell surface, plus two control surfaces (CLI over a unix socket,
settings GUI) that mutate a shared runtime config. Linux / Hyprland
only.

## Vocabulary

- **Daemon** — the `hyprlayd` process: owns the Discord RPC connection,
  the overlay window, and the runtime `Config`. Started explicitly
  (`hyprlay daemon`, direct `hyprlayd`) or automatically: opening the
  settings GUI starts it when it is down, or it runs as the systemd user
  service. Bare `hyprlay` prints help and starts nothing. Closing the
  GUI never stops it — daemon lifetime is never tied to a client's.
- **Client** — any short-lived invocation of the CLI bin (`hyprlay
  <command>`) or the settings GUI bin (`hyprlay-gui`); both send commands
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
- Dependency direction: the three fronts (`cli`, `daemon`, `gui`) are
  modules of one package; each depends only on `hyprlay-core`, and core
  never imports UI-framework or async-runtime types. Inside the daemon
  front: core ← adapters ← overlay/ctl ← shell. Front↔front separation
  is a convention, not a compiler wall: consolidation removed cargo's
  per-crate boundary, so `tests/front_isolation.rs` re-arms it by
  scanning `src/{cli,daemon,gui}` for cross-front imports on every
  `cargo test`. The CLI adds only clap; the GUI adds iced; adapters never
  import UI modules.
- Daemon-side commands (`save`, `dump`, `status`, `help`, `get`,
  `restart`, `quit`, `set monitor`) are answered by the shell before
  reaching `apply_config`.

## Glossary pointers

- Module seams table: [docs/dev/code-layout.md](dev/code-layout.md)
- Logging design (stderr vs JSON wide events): README "Logs"
