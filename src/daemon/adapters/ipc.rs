//! Discord local IPC transport: the raw unix socket next to the client.
//!
//! Discord's websocket bridge (port 6463) sits behind an HTTP upgrade and
//! validates the (client_id, Origin) pair against per-application origins
//! registered in the Developer Portal — an app without a registered origin
//! is silently dropped before any protocol error surfaces. The classic unix
//! socket has no HTTP layer at all, so there is nothing to validate: every
//! properly registered application id connects with zero portal
//! configuration. That gate is exactly why the websocket transport was
//! dropped.
//!
//! Wire format (same as scripts/ipcprobe.rs): 8-byte little-endian header —
//! opcode u32 then payload length u32 — followed by one JSON payload.
//! Opcodes: 0 HANDSHAKE, 1 FRAME (commands/events), 2 CLOSE, 3 PING,
//! 4 PONG.

use std::io;
use std::path::Path;
use std::path::PathBuf;

use serde_json::Value;
use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

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
// Discovery
// ---------------------------------------------------------------------------

/// Socket paths in probe order: the plain layout most clients use first,
/// then Flatpak's per-app runtime dir (and snap's) where sandboxed Discord
/// instances expose their socket.
fn socket_candidates(runtime_dir: &Path) -> Vec<PathBuf> {
    let mut layouts = vec![runtime_dir.to_path_buf()];
    layouts.push(runtime_dir.join("app").join("com.discordapp.Discord"));
    layouts.push(runtime_dir.join("snap.discord"));
    let mut candidates = Vec::with_capacity(layouts.len() * IPC_SLOTS as usize);
    for dir in layouts {
        for slot in 0..IPC_SLOTS {
            candidates.push(dir.join(format!("discord-ipc-{slot}")));
        }
    }
    candidates
}

/// The first candidate that exists on disk; `None` when no client socket
/// is reachable under this runtime dir. Exposed (doc-hidden) so the
/// integration tests can inject their own runtime dir.
#[doc(hidden)]
pub fn discover(runtime_dir: &Path) -> Option<PathBuf> {
    socket_candidates(runtime_dir)
        .into_iter()
        .find(|path| path.exists())
}

// ---------------------------------------------------------------------------
// Stream
// ---------------------------------------------------------------------------

/// A connected Discord IPC socket speaking the framed JSON protocol.
pub struct IpcStream {
    stream: UnixStream,
}

impl IpcStream {
    /// Connect to the first discoverable Discord socket.
    pub async fn connect() -> io::Result<Self> {
        let Some(runtime_dir) = dirs::runtime_dir() else {
            return Err(io::Error::other("no XDG_RUNTIME_DIR set"));
        };
        let Some(path) = discover(&runtime_dir) else {
            return Err(io::Error::other("no discord ipc socket found"));
        };
        let stream = UnixStream::connect(path).await?;
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

    #[test]
    fn candidates_try_plain_layout_before_sandboxed_ones() {
        let dir = PathBuf::from("/run/user/1000");
        let candidates = socket_candidates(&dir);

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

    #[tokio::test]
    async fn ipc_stream_roundtrips_json_values_over_a_real_socket() {
        let (peer, client) = UnixStream::pair().unwrap();
        let mut ipc = IpcStream { stream: client };
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
    async fn handshake_sends_version_and_client_id_on_opcode_zero() {
        let (peer, client) = UnixStream::pair().unwrap();
        let mut ipc = IpcStream { stream: client };

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
    async fn ping_is_answered_with_pong_and_hidden_from_callers() {
        let (mut peer, client) = UnixStream::pair().unwrap();
        let mut ipc = IpcStream { stream: client };

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
    async fn close_frame_ends_the_stream_like_an_eof() {
        let (mut peer, client) = UnixStream::pair().unwrap();
        let mut ipc = IpcStream { stream: client };

        peer.write_all(&frame_bytes(OP_CLOSE, &[])).await.unwrap();

        assert_eq!(ipc.next_frame().await, None);
    }

    #[tokio::test]
    async fn dropped_peer_ends_the_stream_with_none() {
        let (peer, client) = UnixStream::pair().unwrap();
        let mut ipc = IpcStream { stream: client };
        drop(peer);

        assert_eq!(ipc.next_frame().await, None);
    }
}
