# Debug probes

Two throwaway-style probes dump raw Discord RPC traffic. They do not
import any crate code — they speak the wire protocols directly, so they
keep working (and stay truthful) even when the adapters change.

They live in their own mini-crate under `scripts/`, deliberately separate
from the main package, so they never build with or get installed by
`hyprlay`. Their targets are examples, not bins: that keeps the repo's
`cargo install --git` scan at exactly one binary package, so bare
installs work. Run them from that directory:

```sh
cd scripts
cargo run --example ipcprobe      # unix-socket IPC probe (current transport)
cargo run --example wsprobe       # historical websocket bridge probe
```

## Current transport — local IPC (`ipcprobe`)

The daemon talks to Discord over Discord's local unix socket at
`$XDG_RUNTIME_DIR/discord-ipc-N`; the client side lives in
`src/adapters/ipc.rs`. The wire format is 8-byte little-endian framing
(opcode u32 + payload length u32) carrying JSON payloads.

The probe connects to the socket, does the same framing, sends
`HANDSHAKE` then `AUTHORIZE`, and prints every frame for 60s. Use it to
see what a stock Discord client answers and how it frames data.

Start here when Discord changes something: confirm at the protocol level
whether opcodes or payloads moved before touching
`src/adapters/discord.rs`.

The socket has no HTTP layer, so there is no origin validation — any
properly registered application id connects with zero portal
configuration beyond the desktop redirect URI.

## Historical — RPC websocket on port 6463 (`wsprobe`)

An earlier daemon build used the websocket bridge at `ws://127.0.0.1:6463/`.
That bridge sits behind an HTTP upgrade and validates each
`(client_id, Origin)` pair against per-application origins registered in
the Developer Portal; an application without a registered origin is
silently dropped before any protocol error surfaces. That gate is why the
daemon moved to the unix socket. The probe stays for reference only.

It connects to the bridge, waits for the READY dispatch, sends
`AUTHORIZE`, and prints every frame. The optional mode argument controls
the HTTP `Origin` header it sends:

| Mode | Origin sent |
|---|---|
| `no-origin` | none |
| `discord` | `https://discord.com` |
| `tauri` | `tauri://localhost` |
| `dev` | `http://localhost:1420` |
| anything else | sent verbatim as the `Origin` header |
