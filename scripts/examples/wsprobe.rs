//! Debug probe: connect to the local Discord RPC websocket, send AUTHORIZE,
//! and print every frame received. Usage:
//!   cargo run --bin wsprobe [-- no-origin|discord|tauri]

use futures_util::SinkExt;
use futures_util::StreamExt;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::ORIGIN;
use tokio_tungstenite::tungstenite::Message;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "none".into());
    let mut req = "ws://127.0.0.1:6463/?v=1&client_id=123456789012345678"
        .into_client_request()
        .unwrap();
    match mode.as_str() {
        "no-origin" => {}
        "discord" => {
            req.headers_mut()
                .insert(ORIGIN, "https://discord.com".parse().unwrap());
        }
        "tauri" => {
            req.headers_mut()
                .insert(ORIGIN, "tauri://localhost".parse().unwrap());
        }
        "dev" => {
            req.headers_mut()
                .insert(ORIGIN, "http://localhost:1420".parse().unwrap());
        }
        "site" => {
            req.headers_mut()
                .insert(ORIGIN, "https://example.org".parse().unwrap());
        }
        "api" => {
            req.headers_mut()
                .insert(ORIGIN, "https://api.example.org".parse().unwrap());
        }
        other => {
            req.headers_mut().insert(ORIGIN, other.parse().unwrap());
        }
    }

    println!("connecting (origin mode: {mode})…");
    let (mut ws, resp) = match connect_async(req).await {
        Ok(x) => x,
        Err(e) => {
            println!("connect error: {e}");
            return;
        }
    };
    println!("connected, http status: {:?}", resp.status());

    // Wait for the READY dispatch before sending any command — the RPC
    // server ignores frames sent before READY.
    let use_prompt_none = std::env::args().nth(2).as_deref() == Some("none");
    let mut ready = false;
    for _ in 0..2 {
        match tokio::time::timeout(std::time::Duration::from_secs(10), ws.next()).await {
            Ok(Some(Ok(msg))) => {
                println!("[pre] {msg}");
                if msg.to_string().contains("\"evt\":\"READY\"") {
                    ready = true;
                    break;
                }
            }
            _ => break,
        }
    }
    assert!(ready, "no READY dispatch received");
    let hello = if use_prompt_none {
        r#"{"cmd":"AUTHORIZE","args":{"client_id":"123456789012345678","scopes":["identify","rpc"],"prompt":"none"},"nonce":"probe-1"}"#
    } else {
        r#"{"cmd":"AUTHORIZE","args":{"client_id":"123456789012345678","scopes":["identify","rpc"]},"nonce":"probe-1"}"#
    };
    ws.send(Message::Text(hello.into())).await.unwrap();
    println!("sent AUTHORIZE (prompt none = {use_prompt_none}) — watch Discord for the modal");

    for i in 0..8 {
        match tokio::time::timeout(std::time::Duration::from_secs(60), ws.next()).await {
            Ok(Some(Ok(msg))) => println!("[{i}] {msg}"),
            Ok(Some(Err(e))) => {
                println!("[{i}] ws error: {e}");
                break;
            }
            Ok(None) => {
                println!("[{i}] stream closed by server");
                break;
            }
            Err(_) => {
                println!("[{i}] timeout waiting for frame");
                break;
            }
        }
    }
}
