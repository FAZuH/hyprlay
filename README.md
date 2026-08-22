# hyprlay

A lightweight Discord voice overlay for Hyprland.

## Install

### Prebuilt binary

Download the binaries at `https://github.com/FAZuH/hyprlay/releases/latest`
and move it all to `$PATH`

### Cargo

Build & install with cargo (`curl https://sh.rustup.rs -sSf | sh`)

```sh
cargo install --git https://github.com/FAZuH/hyprlay
```

Or build from a cloned checkout and copy manually:

```sh
cargo build --release
```

Copy all three binaries from `target/release/` into one directory on
your `$PATH`, for example `~/.local/bin`.

## Run

Bare `hyprlay` prints the help and exits 0. Start the daemon
explicitly:

```sh
hyprlay daemon   # or invoke the hyprlayd binary directly
```

Opening the settings window (`hyprlay gui`) starts the daemon too when
it is down; closing the window leaves it running. Under systemd,
`hyprlay install` keeps it alive as a user service (see Service).

hyprlay signs in through your own Discord application. Create one in the
Discord Developer Portal and give its client id and client secret to
hyprlay — see [docs/token-exchange.md](docs/token-exchange.md).

On first use, Discord shows an approval modal — click **Authorize**
once. The token is cached at `~/.config/hyprlay/token.json` (mode 0600).
Your client secret stays in `auth.json` or the environment. It never
appears in logs.

A second daemon instance refuses to start: it prints an error that names
the socket and exits with status 1. Autostart lines that launch the
daemon twice are unaffected — the loser just exits.

### Mouse

The overlay never catches clicks. Every pointer event passes through to
the windows below it. Hold the left button to drag the overlay to a new
spot.

## CLI

`daemon` runs the overlay; bare `hyprlay` prints the help. Any other
word turns the binary into a client of the running daemon over
`$XDG_RUNTIME_DIR/hyprlay.sock`. Help and version are answered locally,
without a running daemon.

Help has a short shape: `hyprlay -h` lists every command with a
one-line description, plus FILES and EXAMPLES sections. Every command
answers its own help too: `hyprlay <command> -h`. The full KEYS table
lives only under `set -h` and `get -h`; both generate it from the same
source as the wire `help` reply, so the two cannot drift. Terse
synopses keep the table readable — `move -h` spells out its positions.

| Command | Action |
|---|---|
| `gui` | open the settings window |
| `status` | connection + config summary |
| `get <key>` | read one setting |
| `set <key> [value]` | change one setting (a bare `set <key>` cycles enums and flips flags) |
| `move <pos>` | re-anchor to a screen edge (`left\|right\|center\|top\|bottom`) |
| `nudge <dx> <dy>` | shift the overlay by pixels |
| `reset [section]` | reset all groups or one (`position\|layout\|opacity\|colors`; always keeps the monitor) |
| `save` | write `config.toml` now |
| `reload` | re-read `config.toml` |
| `restart` | re-exec the daemon (applies new credentials) |
| `quit` | stop the running daemon cleanly |
| `monitors` | list outputs |
| `dump` | print the live runtime config as TOML |
| `install [--no-start]` | install the systemd user service + desktop entry |
| `uninstall` | remove the service + desktop entry again |

`monitors` never contacts the daemon either — output detection happens
in-process. Bad input fails locally too: unknown commands, missing
arguments, and unknown values are rejected before anything touches the
socket.

Examples:

```sh
hyprlay gui                  # settings window
hyprlay status               # connection + config summary
hyprlay set position         # cycle corner presets
hyprlay set anchor bottom    # glue edge: auto | top | bottom
hyprlay set rtl on           # avatar on the right, text right-aligned
hyprlay set offset-x 16      # px from the anchored screen edge
hyprlay nudge 20 0           # dx dy in px (negative allowed)
hyprlay set opacity 70       # 0-100
hyprlay set width 340        # 200-600
hyprlay set scale 125        # 50-200 %
hyprlay set visible off      # hide all roster rows
hyprlay set talking-only on  # render speaking users only
hyprlay monitors             # list outputs
hyprlay set monitor eDP-1    # relocate the overlay to that output
hyprlay reset                # back to defaults (keeps the monitor)
```

Every value is clamped to its bounds and applies live.

Persistence is daemon-owned. With `auto-save` on (the default), the
daemon writes `config.toml` after every successful change. With
`auto-save` off, changes stay session-only until you run
`hyprlay save`.

A monitor change re-creates the layer surface, so `set monitor`
restarts the daemon. An unknown output name fails with a clean error.
A bare `set monitor` cycles from the active monitor through each
detected output.

Nudges shift the overlay at runtime only: re-anchoring or restarting
resets the shift (`nudge -h` says so too). Persistent shifts belong in
the config as `offset-x` / `offset-y`.

Keys (use with `get`/`set`):

- Position: `position`, `anchor`, `monitor`, `offset-x`, `offset-y`,
  `offset-min`, `offset-max`, `rtl`
- Layout: `width`, `scale`, `avatar-size`, `text-size`, `spacing`,
  `max-name`, `talking-only`, `own-user`, `visible`, `auto-save`
- Opacity: `opacity`, `avatar-opacity`, `text-opacity`, `box-opacity`
- Colors: `speaking-color`, `text-color`, `box-color`

Hyprland keybinds:

```text
bind = SUPER, F9,  exec, hyprlay set visible
bind = SUPER, F10, exec, hyprlay set talking-only
bind = SUPER, F11, exec, hyprlay set position
```

## Service

`hyprlay install` turns the daemon into a systemd **user** service and
adds a desktop entry. It writes:

- `~/.config/systemd/user/hyprlay.service` — starts the `hyprlayd`
  next to the running binary, with `Restart=on-failure`
- `~/.local/share/applications/hyprlay.desktop` — menu entry that
  opens the settings window

It then runs `systemctl --user daemon-reload` followed by
`systemctl --user enable --now hyprlay`. Pass `--no-start` to write
both files but leave the service disabled. Running `install` again is
safe — it overwrites both files.

`hyprlay uninstall` stops and disables the service (a missing unit is
fine) and removes both files. It succeeds identically when nothing is
installed.

Credentials under systemd: environment variables do not reach a user
service unless the unit declares them, so the unit carries
`EnvironmentFile=-%h/.config/hyprlay/service.env`. The leading dash
makes the file optional; put env-based credentials there, one
`KEY=value` per line. The simpler path needs no file: sign in once
through the settings window. The GUI stores `auth.json` in
`~/.config/hyprlay/`, which the daemon reads no matter how it was
started. See [docs/token-exchange.md](docs/token-exchange.md).

## Config

`~/.config/hyprlay/config.toml` (created on first change, clamped on
load). The four sections mirror the settings window:

```toml
[position]
horizontal = "left"              # left | right | center
vertical = "top"                 # top | bottom
anchor = "auto"                  # auto | top | bottom
offset-x = 12                    # px from the anchored screen edges
offset-y = 12
offset-min = -2000               # lower bound for the offset inputs
offset-max = 2000                # upper bound for the offset inputs
rtl = false                      # avatar on the right, text right-aligned
# monitor = "eDP-1"              # target output; unset = active monitor

[layout]
width = 300                      # panel width, 200-600
scale = 100                      # 50-200 %
avatar-size = 34                 # 16-64
text-size = 14                   # 8-32
spacing = 4                      # gap between rows, 0-24
max-name = 16                    # username length cap, 4-64
talking-only = false             # render speaking users only
own-user = true                  # show yourself
visible = true                   # false hides all roster rows
auto-save = true                 # persist every applied change

[opacity]
overall = 100                    # 0-100
avatar = 100                     # 0-100, applied on top of overall
text = 100                       # 0-100, applied on top of overall
box = 90                         # username chip background, 0-100

[colors]
speaking = "#22c55e"             # speaking ring
text = "#ffffff"                 # username text
box = "#0d0d0f"                  # username chip background
```

## Logs

Two streams, by design:

- **stderr (user-facing):** compact human-readable lines for lifecycle
  events (connected, authenticated, channel joined) and errors. Library
  noise (wgpu, layershellev) never shows up here.
- **`$XDG_STATE_HOME/hyprlay/logs/` (machine-facing):** one JSON
  wide event per unit of work, daily rotation. An `rpc_session` span
  per IPC connection carries the outcome, auth state, and error codes.
  Reconnects, token exchange failures, and channel switches are all
  individual events. Tokens and OAuth codes are never logged.

Verbosity of the file stream is controlled by `RUST_LOG` (default `info`):

```sh
RUST_LOG=debug hyprlayd
```

## Docs

User guide:

- [docs/token-exchange.md](docs/token-exchange.md) — connect hyprlay to
  your own Discord application

Developer docs:

- [docs/dev/code-layout.md](docs/dev/code-layout.md) — workspace layout,
  module seams, and layering rules
- [docs/dev/debug-probes.md](docs/dev/debug-probes.md) — the `ipcprobe`
  / `wsprobe` examples for raw Discord RPC traffic

## License

MIT
