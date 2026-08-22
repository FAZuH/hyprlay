//! Daemon-side control socket: accepts connections forever and turns each
//! command line into a [`CtlRequest`] the shell can answer. The wire
//! vocabulary (paths, probe, reply format) lives in `hyprlay-core::ctl`;
//! only this tokio listener is daemon-specific.

use tokio::sync::oneshot;

/// One inbound command line plus the channel to answer it on. The shell
/// parses, applies, and sends exactly one reply string.
pub struct CtlRequest {
    pub command: String,
    pub reply: oneshot::Sender<String>,
}

/// Daemon side of the control socket as an iced subscription stream.
pub fn incoming() -> impl futures_util::Stream<Item = CtlRequest> {
    use futures_util::SinkExt;
    iced::stream::channel(
        16,
        |sender: futures_channel::mpsc::Sender<CtlRequest>| async move {
            let path = hyprlay_core::ctl::socket_path();
            // The startup probe already unlinked any stale socket before we
            // got here. No unconditional unlink at this point: in the rare
            // probe/bind race it could delete a live daemon's fresh socket.
            let listener = match tokio::net::UnixListener::bind(&path) {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!(event = "ctl_socket_bind_failed", error = %e, "control socket unavailable");
                    return;
                }
            };
            tracing::info!(event = "ctl_socket_listening", path = %path.display(), "control socket ready");
            loop {
                let Ok((conn, _)) = listener.accept().await else {
                    continue;
                };
                let mut sender = sender.clone();
                tokio::spawn(async move {
                    use tokio::io::AsyncBufReadExt;
                    use tokio::io::AsyncWriteExt;
                    use tokio::io::BufReader;
                    let (read_half, mut write_half) = conn.into_split();
                    let mut line = String::new();
                    let mut reader = BufReader::new(read_half);
                    if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                        return;
                    }
                    let (reply_tx, reply_rx) = oneshot::channel();
                    if sender
                        .send(CtlRequest {
                            command: line.trim().to_string(),
                            reply: reply_tx,
                        })
                        .await
                        .is_ok()
                        && let Ok(reply) = reply_rx.await
                    {
                        let _ = write_half.write_all(reply.as_bytes()).await;
                        let _ = write_half.write_all(b"\n").await;
                    }
                });
            }
        },
    )
}
