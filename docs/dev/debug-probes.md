# Debug probes

Three probes live here. Two dump raw Discord RPC traffic; they do not
import any crate code — they speak the wire protocols directly, so they
keep working (and stay truthful) even when the adapters change. The
third is a plain shell sampler for RSS-hunting on a process that does
not look like a leak in the code but does in btop.

The two Discord probes live in their own mini-crate under `scripts/`,
deliberately separate from the main package, so they never build with or
get installed by `hyprlay`. Their targets are examples, not bins: that
keeps the repo's `cargo install --git` scan at exactly one binary
package, so bare installs work. Run them from that directory:

```sh
cd scripts
cargo run --example ipcprobe      # unix-socket IPC probe (current transport)
cargo run --example wsprobe       # historical websocket bridge probe
```

## Daemon memory sampler (`rssprobe.sh`)

`scripts/rssprobe.sh` samples one process's memory every couple of
seconds and appends one CSV row per interval:
`epoch,timestamp,rss_kb,hwm_kb,note`. It reads `/proc/<pid>/status`
directly — VmRSS plus the peak VmHWM — so every row shows both the
live footprint and the historical high-water mark. That pair is the
diagnosis signal:

- **RSS at or near HWM** — the process is at its historical peak;
  what you are watching is real growth.
- **RSS well below HWM** — memory was freed but not returned to the
  OS (allocator retention). The interesting question becomes *what
  spiked it earlier*, not *what is growing now*.

Usage — sampler in one terminal, markers from another while you
work:

```sh
scripts/rssprobe.sh                          # sample `pidof hyprlayd` every 2s
scripts/rssprobe.sh -p <pid> -i 5 -o d.csv   # explicit target/interval/output
scripts/rssprobe.sh mark "VC join"           # append a labeled marker row
scripts/rssprobe.sh mark -o d.csv "VC join"  # marker into the same CSV
```

Ctrl-C stops the sampler. Both commands append to the same CSV
(`rssprobe.csv` in the CWD by default; the header is written once),
so a later plot or diff shows exactly which scenario moved the
needle.

Runbook for the hyprlayd memory regression (spec: v031 memory, H1–H5):

1. Cold idle: start the sampler on a freshly restarted daemon, mark
   `idle-start`, wait 10 min, mark `idle-end`.
2. H1 bait: mark `window-move-storm`, sweep a window across the
   overlay with `dim-on-hover` on (the poll path opens a fresh
   Hyprland socket per 50ms tick while connected with a non-empty
   roster), then repeat with it off.
3. VC join/leave cycles, camera/stream toggles: mark each; watch RSS
   vs HWM after the leave.
4. Repeat the identical series on the v0.3.0 release build (`hyprlayd`
   from that tag) and diff the CSVs.

## Current transport — local IPC (`ipcprobe`)

The daemon talks to Discord over Discord's local unix socket at
`$XDG_RUNTIME_DIR/discord-ipc-N`; the client side lives in
`src/daemon/adapters/ipc.rs`. The wire format is 8-byte little-endian
framing (opcode u32 + payload length u32) carrying JSON payloads.

The probe connects to the socket, does the same framing, sends
`HANDSHAKE` then `AUTHORIZE`, and prints every frame for 60s. Use it to
see what a stock Discord client answers and how it frames data.

Start here when Discord changes something: confirm at the protocol level
whether opcodes or payloads moved before touching
`src/daemon/adapters/discord.rs`.

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
