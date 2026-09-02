//! Discord local IPC transport: the raw socket/pipe next to the client.
//!
//! Discord's websocket bridge (port 6463) sits behind an HTTP upgrade and
//! validates the (client_id, Origin) pair against per-application origins
//! registered in the Developer Portal — an app without a registered origin
//! is silently dropped before any protocol error surfaces. The classic unix
//! socket (and Windows named pipe) has no HTTP layer at all, so there is
//! nothing to validate: every properly registered application id connects
//! with zero portal configuration. That gate is exactly why the websocket
//! transport was dropped.
//!
//! Wire format (same as scripts/ipcprobe.rs): 8-byte little-endian header —
//! opcode u32 then payload length u32 — followed by one JSON payload.
//! Opcodes: 0 HANDSHAKE, 1 FRAME (commands/events), 2 CLOSE, 3 PING,
//! 4 PONG.
//!
//! The codec ([`frame_bytes`], [`split_header`], the opcodes, the payload cap)
//! is pure and platform-free. The stream [`IpcStream`] is transport-agnostic:
//! it runs over any [`DiscordIo`] byte stream. Only discovery + connect are
//! per-OS, behind the [`DiscordTransport`] port — a unix socket on
//! Linux/macOS, a named pipe on Windows.

use std::io;
use std::path::Path;
use std::path::PathBuf;

use serde_json::Value;
use serde_json::json;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;

pub const OP_HANDSHAKE: u32 = 0;
pub const OP_FRAME: u32 = 1;
pub const OP_CLOSE: u32 = 2;
pub const OP_PING: u32 = 3;
pub const OP_PONG: u32 = 4;

/// Upper bound for one payload. Real Discord payloads are tiny JSON docs;
/// the cap only stops a corrupt length field from allocating gigabytes.
const MAX_PAYLOAD_BYTES: u32 = 16 * 1024 * 1024;

/// How many socket numbers to probe per layout, matching ecosystem tools.
const IPC_SLOTS: u32 = 10;

// ---------------------------------------------------------------------------
// Codec (pure, unit-testable without sockets)
// ---------------------------------------------------------------------------

/// Serialize one frame: LE opcode + LE length + body.
fn frame_bytes(opcode: u32, body: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8 + body.len());
    buf.extend_from_slice(&opcode.to_le_bytes());
    buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
    buf.extend_from_slice(body);
    buf
}

struct FrameHeader {
    opcode: u32,
    length: u32,
}

fn split_header(header: [u8; 8]) -> FrameHeader {
    FrameHeader {
        opcode: u32::from_le_bytes(header[0..4].try_into().expect("header is 8 bytes")),
        length: u32::from_le_bytes(header[4..8].try_into().expect("header is 8 bytes")),
    }
}

// ---------------------------------------------------------------------------
// Stream (transport-agnostic)
// ---------------------------------------------------------------------------

/// A byte stream the Discord framed-JSON protocol runs over. Boxed behind this
/// trait so the OS transport (unix socket vs named pipe) can vary without
/// touching the protocol logic. Only one non-auto trait may appear in a trait
/// object, so this collects `AsyncRead + AsyncWrite + Unpin + Send` into one.
pub trait DiscordIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> DiscordIo for T {}

/// A connected Discord IPC socket speaking the framed JSON protocol. The
/// protocol behavior is shared across every OS; only the transport differs.
pub struct IpcStream {
    stream: Box<dyn DiscordIo>,
}

impl IpcStream {
    /// Connect to the first discoverable Discord IPC endpoint for this OS.
    pub async fn connect() -> io::Result<Self> {
        let stream = Discord.connect().await?;
        Ok(Self { stream })
    }

    pub async fn handshake(&mut self, client_id: &str) -> io::Result<()> {
        let body = serde_json::to_vec(&json!({ "v": 1, "client_id": client_id }))
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.stream
            .write_all(&frame_bytes(OP_HANDSHAKE, &body))
            .await
    }

    /// Send one command/event payload as a FRAME.
    pub async fn send(&mut self, payload: &Value) -> io::Result<()> {
        let body = serde_json::to_vec(payload)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        self.stream.write_all(&frame_bytes(OP_FRAME, &body)).await
    }

    /// Next caller-visible frame, or `None` once the connection is gone.
    ///
    /// Non-FRAME opcodes are handled here so callers only ever see JSON:
    /// PING is answered with PONG immediately (Discord uses it as a
    /// liveness probe; ignoring it risks the pipe being reaped), CLOSE ends
    /// the stream like an EOF, and anything else is control noise.
    pub async fn next_frame(&mut self) -> Option<Value> {
        loop {
            let mut header = [0u8; 8];
            match self.stream.read_exact(&mut header).await {
                Ok(_) => {}
                // A clean close and a hard error both end the session; the
                // reconnect loop treats them identically either way.
                Err(_) => return None,
            }
            let FrameHeader { opcode, length } = split_header(header);
            if length > MAX_PAYLOAD_BYTES {
                // Corrupt or hostile framing: no way to resync reliably.
                return None;
            }
            let mut body = vec![0u8; length as usize];
            if self.stream.read_exact(&mut body).await.is_err() {
                return None;
            }
            match opcode {
                OP_FRAME => match serde_json::from_slice(&body) {
                    Ok(v) => return Some(v),
                    // One bad payload must not kill the session; skip it
                    // like the old websocket code skipped bad text frames.
                    Err(_) => continue,
                },
                OP_PING => {
                    // Echo the ping body straight back as PONG.
                    let pong = frame_bytes(OP_PONG, &body);
                    self.stream.write_all(&pong).await.ok();
                }
                OP_CLOSE => return None,
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Transport port (per-OS discovery + connect)
// ---------------------------------------------------------------------------

/// Transport port for the Discord local IPC channel. Discovery + connect are
/// the only per-OS pieces; the protocol runs over whatever byte stream this
/// produces.
pub trait DiscordTransport {
    fn connect(&self) -> impl std::future::Future<Output = io::Result<Box<dyn DiscordIo>>> + Send;
}

/// The daemon's Discord transport adapter (unit struct — discovery and connect
/// are stateless per OS).
pub struct Discord;

#[cfg(unix)]
impl DiscordTransport for Discord {
    fn connect(&self) -> impl std::future::Future<Output = io::Result<Box<dyn DiscordIo>>> + Send {
        unix::connect()
    }
}

#[cfg(windows)]
impl DiscordTransport for Discord {
    fn connect(&self) -> impl std::future::Future<Output = io::Result<Box<dyn DiscordIo>>> + Send {
        windows::connect()
    }
}

// ---------------------------------------------------------------------------
// Discovery — unix (Linux/macOS)
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod unix {
    use super::*;

    /// Socket paths in probe order for one base dir: the plain layout most
    /// clients use first, then Flatpak's per-app runtime dir (and snap's)
    /// where sandboxed Discord instances expose their socket.
    pub(super) fn socket_candidates(base: &Path) -> Vec<PathBuf> {
        let mut layouts = vec![base.to_path_buf()];
        layouts.push(base.join("app").join("com.discordapp.Discord"));
        layouts.push(base.join("snap.discord"));
        let mut candidates = Vec::with_capacity(layouts.len() * IPC_SLOTS as usize);
        for dir in layouts {
            for slot in 0..IPC_SLOTS {
                candidates.push(dir.join(format!("discord-ipc-{slot}")));
            }
        }
        candidates
    }

    /// The first candidate that exists on disk; `None` when no client socket
    /// is reachable under this base dir. Exposed (doc-hidden) so the
    /// integration tests can inject their own runtime dir.
    #[doc(hidden)]
    pub fn discover(base: &Path) -> Option<PathBuf> {
        socket_candidates(base)
            .into_iter()
            .find(|path| path.exists())
    }

    /// Pure macOS-style fallback chain over explicit inputs (env-independent so
    /// it is unit-testable): runtime dir first, then TMPDIR → TMP → TEMP →
    /// /tmp, deduped in order.
    #[cfg(any(not(target_os = "linux"), test))]
    pub(super) fn fallback_bases(
        runtime: Option<PathBuf>,
        tmpdir: Option<PathBuf>,
        tmp: Option<PathBuf>,
        temp: Option<PathBuf>,
        fallback: PathBuf,
    ) -> Vec<PathBuf> {
        use std::collections::HashSet;

        let mut dirs = Vec::new();
        if let Some(d) = runtime {
            dirs.push(d);
        }
        if let Some(d) = tmpdir {
            dirs.push(d);
        }
        if let Some(d) = tmp {
            dirs.push(d);
        }
        if let Some(d) = temp {
            dirs.push(d);
        }
        dirs.push(fallback);
        let mut seen = HashSet::new();
        dirs.retain(|d| seen.insert(d.clone()));
        dirs
    }

    /// Base dirs to probe, in order. Linux keeps the XDG runtime dir; macOS
    /// (no `runtime_dir`) falls back through TMPDIR → TMP → TEMP → /tmp —
    /// probing each base's candidates for *existence* rather than stopping at
    /// the first defined env var (the macOS bug class is a user-exported
    /// `XDG_RUNTIME_DIR`/`TMPDIR` pointing at the wrong place).
    fn discovery_bases() -> Vec<PathBuf> {
        #[cfg(target_os = "linux")]
        {
            let mut dirs = Vec::new();
            if let Some(d) = dirs::runtime_dir() {
                dirs.push(d);
            }
            dirs
        }
        #[cfg(not(target_os = "linux"))]
        {
            fallback_bases(
                dirs::runtime_dir(),
                std::env::var_os("TMPDIR").map(PathBuf::from),
                std::env::var_os("TMP").map(PathBuf::from),
                std::env::var_os("TEMP").map(PathBuf::from),
                std::env::temp_dir(),
            )
        }
    }

    /// Connect to the first discoverable Discord UDS socket.
    pub(super) async fn connect() -> io::Result<Box<dyn DiscordIo>> {
        for base in discovery_bases() {
            if let Some(path) = discover(&base) {
                return Ok(Box::new(tokio::net::UnixStream::connect(path).await?));
            }
        }
        Err(io::Error::other("no discord ipc socket found"))
    }
}

/// Re-export for the integration tests that drive discovery over an injected
/// temp dir.
#[cfg(unix)]
pub use unix::discover;

// ---------------------------------------------------------------------------
// Discovery — Windows (named pipes)
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod windows {
    use interprocess::local_socket::GenericNamespaced;
    use interprocess::local_socket::Name;
    use interprocess::local_socket::tokio::prelude::*;

    use super::*;

    /// The `\\.\pipe\discord-ipc-{0..9}` candidate names, per slot. Pure — a
    /// `GenericNamespaced` name prepends the pipe prefix for us.
    pub(super) fn pipe_candidates() -> Vec<String> {
        (0..IPC_SLOTS)
            .map(|slot| format!("discord-ipc-{slot}"))
            .collect()
    }

    /// The namespaced pipe name for one slot.
    fn pipe_name(slot: u32) -> io::Result<Name<'static>> {
        Ok(format!("discord-ipc-{slot}")
            .to_ns_name::<GenericNamespaced>()?
            .into_owned())
    }

    /// Connect to the first named pipe that accepts a connection.
    pub(super) async fn connect() -> io::Result<Box<dyn DiscordIo>> {
        for slot in 0..IPC_SLOTS {
            if let Ok(stream) = LocalSocketStream::connect(pipe_name(slot)?).await {
                return Ok(Box::new(stream));
            }
        }
        Err(io::Error::other("no discord ipc pipe found"))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_encoding_matches_the_8_byte_le_wire_format() {
        // Hand-built expectation: opcode 1, body "hi" (len 2).
        let bytes = frame_bytes(OP_FRAME, b"hi");
        assert_eq!(
            bytes,
            vec![0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, b'h', b'i']
        );
    }

    #[test]
    fn header_split_recovers_opcode_and_length() {
        let header = [0u8, 0, 0, 0, 42, 0, 0, 0];
        let parsed = split_header(header);
        assert_eq!(parsed.opcode, OP_HANDSHAKE);
        assert_eq!(parsed.length, 42);
    }

    #[test]
    fn frame_roundtrip_preserves_multi_byte_utf8_payloads() {
        // Lengths must count UTF-8 bytes, not chars: this body mixes two-
        // byte, four-byte, and three-byte characters.
        let payload = Value::String("héllo 🎉 你好".to_string());
        let body = serde_json::to_vec(&payload).unwrap();

        let frame = frame_bytes(OP_FRAME, &body);
        let (head, tail) = frame.split_at(8);
        let parsed = split_header(head.try_into().unwrap());
        assert_eq!(parsed.opcode, OP_FRAME);
        assert_eq!(parsed.length as usize, body.len());

        let back: Value = serde_json::from_slice(tail).unwrap();
        assert_eq!(back, payload);
    }

    #[test]
    fn empty_payload_frames_are_well_formed() {
        let frame = frame_bytes(OP_CLOSE, b"");
        assert_eq!(&frame[..8], &[2, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(frame.len(), 8);
    }

    #[cfg(unix)]
    #[test]
    fn candidates_try_plain_layout_before_sandboxed_ones() {
        let dir = PathBuf::from("/run/user/1000");
        let candidates = unix::socket_candidates(&dir);

        assert_eq!(candidates[0], PathBuf::from("/run/user/1000/discord-ipc-0"));
        assert_eq!(candidates[9], PathBuf::from("/run/user/1000/discord-ipc-9"));
        assert_eq!(
            candidates[10],
            PathBuf::from("/run/user/1000/app/com.discordapp.Discord/discord-ipc-0")
        );
        assert_eq!(
            candidates[20],
            PathBuf::from("/run/user/1000/snap.discord/discord-ipc-0")
        );
    }

    #[cfg(unix)]
    #[test]
    fn fallback_bases_deduplicate_and_keep_probe_order() {
        let bases = unix::fallback_bases(
            Some(PathBuf::from("/run/user/1000")),
            Some(PathBuf::from("/var/folders/ab/T")),
            None,
            Some(PathBuf::from("/var/folders/ab/T")),
            PathBuf::from("/tmp"),
        );
        // runtime first, then TMPDIR, then the default; the duplicate TEMP
        // value collapses into TMPDIR and the order is preserved.
        assert_eq!(
            bases,
            vec![
                PathBuf::from("/run/user/1000"),
                PathBuf::from("/var/folders/ab/T"),
                PathBuf::from("/tmp"),
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn pipe_candidates_spans_the_ten_slots() {
        let names = windows::pipe_candidates();
        assert_eq!(names.len(), 10);
        assert_eq!(names[0], "discord-ipc-0");
        assert_eq!(names[9], "discord-ipc-9");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn ipc_stream_roundtrips_json_values_over_a_real_socket() {
        let (peer, client) = tokio::net::UnixStream::pair().unwrap();
        let mut ipc = IpcStream {
            stream: Box::new(client),
        };
        let payload = json!({ "cmd": "SUBSCRIBE", "args": {}, "nonce": "ovl-1" });

        ipc.send(&payload).await.unwrap();

        let mut reader = peer;
        let mut header = [0u8; 8];
        reader.read_exact(&mut header).await.unwrap();
        let parsed = split_header(header);
        assert_eq!(parsed.opcode, OP_FRAME);
        let mut body = vec![0u8; parsed.length as usize];
        reader.read_exact(&mut body).await.unwrap();
        assert_eq!(serde_json::from_slice::<Value>(&body).unwrap(), payload);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn handshake_sends_version_and_client_id_on_opcode_zero() {
        let (peer, client) = tokio::net::UnixStream::pair().unwrap();
        let mut ipc = IpcStream {
            stream: Box::new(client),
        };

        ipc.handshake("123456789012345678").await.unwrap();

        let mut reader = peer;
        let mut header = [0u8; 8];
        reader.read_exact(&mut header).await.unwrap();
        let parsed = split_header(header);
        assert_eq!(parsed.opcode, OP_HANDSHAKE);
        let mut body = vec![0u8; parsed.length as usize];
        reader.read_exact(&mut body).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["v"], 1);
        assert_eq!(v["client_id"], "123456789012345678");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn ping_is_answered_with_pong_and_hidden_from_callers() {
        let (mut peer, client) = tokio::net::UnixStream::pair().unwrap();
        let mut ipc = IpcStream {
            stream: Box::new(client),
        };

        // Discord pings us...
        let ping_body = serde_json::to_vec(&json!({ "nonce": "rpc-ping-1" })).unwrap();
        peer.write_all(&frame_bytes(OP_PING, &ping_body))
            .await
            .unwrap();
        // ...and keeps sending real frames afterwards.
        let event = json!({ "evt": "READY" });
        peer.write_all(&frame_bytes(OP_FRAME, &serde_json::to_vec(&event).unwrap()))
            .await
            .unwrap();

        // The ping itself never surfaces; the caller sees only the frame.
        assert_eq!(ipc.next_frame().await, Some(event));

        // And the peer received our automatic PONG echo, not silence.
        let mut header = [0u8; 8];
        peer.read_exact(&mut header).await.unwrap();
        let pong = split_header(header);
        assert_eq!(pong.opcode, OP_PONG);
        let mut body = vec![0u8; pong.length as usize];
        peer.read_exact(&mut body).await.unwrap();
        assert_eq!(body, ping_body);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn close_frame_ends_the_stream_like_an_eof() {
        let (mut peer, client) = tokio::net::UnixStream::pair().unwrap();
        let mut ipc = IpcStream {
            stream: Box::new(client),
        };

        peer.write_all(&frame_bytes(OP_CLOSE, &[])).await.unwrap();

        assert_eq!(ipc.next_frame().await, None);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn dropped_peer_ends_the_stream_with_none() {
        let (peer, client) = tokio::net::UnixStream::pair().unwrap();
        let mut ipc = IpcStream {
            stream: Box::new(client),
        };
        drop(peer);

        assert_eq!(ipc.next_frame().await, None);
    }
}
