//! Debug probe for the classic unix-socket RPC protocol:
//!   $XDG_RUNTIME_DIR/discord-ipc-N, 8-byte LE framing (opcode u32 + len u32).
//! Sends HANDSHAKE then AUTHORIZE and prints every frame for 60s.

use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;

async fn send_frame(
    stream: &mut UnixStream,
    opcode: u32,
    payload: &serde_json::Value,
) -> std::io::Result<()> {
    let body = serde_json::to_string(payload).unwrap();
    let mut buf = Vec::with_capacity(8 + body.len());
    buf.extend_from_slice(&opcode.to_le_bytes());
    buf.extend_from_slice(&(body.len() as u32).to_le_bytes());
    buf.extend_from_slice(body.as_bytes());
    stream.write_all(&buf).await
}

async fn read_frame(stream: &mut UnixStream) -> std::io::Result<Option<(u32, String)>> {
    let mut header = [0u8; 8];
    match stream.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let opcode = u32::from_le_bytes(header[0..4].try_into().unwrap());
    let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;
    Ok(Some((opcode, String::from_utf8_lossy(&body).into_owned())))
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR");
    let mut stream = None;
    for i in 0..10 {
        let path = format!("{runtime_dir}/discord-ipc-{i}");
        if let Ok(s) = UnixStream::connect(&path).await {
            println!("connected to {path}");
            stream = Some(s);
            break;
        }
    }
    let mut stream = match stream {
        Some(s) => s,
        None => {
            println!("no discord-ipc socket found");
            return;
        }
    };

    let handshake = serde_json::json!({
        "v": 1,
        "client_id": "123456789012345678",
    });
    send_frame(&mut stream, 0, &handshake).await.unwrap();
    println!("sent HANDSHAKE");

    // Skip OAuth entirely: try voice access with the plain handshake.
    let sub = serde_json::json!({
        "cmd": "NOT_A_REAL_COMMAND",
        "args": {},
        "nonce": "probe-garbage-1",
    });
    send_frame(&mut stream, 1, &sub).await.unwrap();
    println!("sent SUBSCRIBE VOICE_CHANNEL_SELECT");

    let get_sel = serde_json::json!({
        "cmd": "GET_SELECTED_VOICE_CHANNEL",
        "args": {},
        "nonce": "probe-sel-1",
    });
    send_frame(&mut stream, 1, &get_sel).await.unwrap();
    println!("sent GET_SELECTED_VOICE_CHANNEL");

    for i in 0..10 {
        match tokio::time::timeout(std::time::Duration::from_secs(60), read_frame(&mut stream))
            .await
        {
            Ok(Ok(Some((opcode, body)))) => println!("[{i}] opcode={opcode} {body}"),
            Ok(Ok(None)) => {
                println!("[{i}] socket closed");
                break;
            }
            Ok(Err(e)) => {
                println!("[{i}] io error: {e}");
                break;
            }
            Err(_) => {
                println!("[{i}] timeout waiting for frame");
                break;
            }
        }
    }
}
