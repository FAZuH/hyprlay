# hyprlay

**A lightweight Discord voice overlay for Hyprland.**

<hr>

<div align="center">
● <a href="#installation">Installation</a> ﻿ ● <a href="#preview">Preview</a> ﻿ ● <a href="#usage">Usage</a> ﻿ ● <a href="#docs">Docs</a> ﻿ ● <a href="#license">License</a>
</div>

## Installation

### Prebuilt binary

Download the latest binaries from [Releases](https://github.com/FAZuH/hyprlay/releases/latest) and copy all four (`hyprlay`, `hyprlayd`, `hyprlay-gui`, `hyprlay-tray`) to a directory on your `$PATH` (e.g. `~/.local/bin`).

### Cargo

```sh
cargo install --git https://github.com/FAZuH/hyprlay
```

Or build from source:

```sh
cargo build --release
# then copy target/release/hyprlay{,d,-gui,-tray} to $PATH
```

## Setup

1. Follow [Token Exchange](docs/token-exchange.md) guide to connect your Discord account with the app.
2. Install the app with `hyprlay install`.

## Usage

```sh
hyprlay gui                 # open settings — starts daemon
hyprlay status              # connection + config summary
hyprlay set position        # cycle screen corners
hyprlay set visible         # toggle roster visibility
hyprlay set monitor eDP-1   # move to output
hyprlayd                    # start overlay directly
```

See `hyprlay -h` and `hyprlay set -h` for all commands and keys.md) for Discord auth.

## Docs

- [Token Exchange](docs/token-exchange.md) — connect hyprlay to your own Discord application
- [Code Layout](docs/dev/code-layout.md) — workspace layout, module interfaces, and layering rules
- [Debug Probes](docs/dev/debug-probes.md) — the `ipcprobe` / `wsprobe` examples for raw Discord RPC traffic
- [Changelog Guide](docs/dev/changelog.md) — Keep a Changelog style and scope grouping
- [Commit Scopes](docs/dev/commit-scopes.md) — allowed `type(scope)` values

## License

MIT
