//! Daemon-side control socket: accepts connections forever and turns each
//! command line into a [`CtlRequest`] the shell can answer. The wire
//! vocabulary (paths, probe, reply format) lives in `hyprlay-core::ctl`;
//! only the accept loop is daemon-specific.
//!
//! The listener runs on its own thread so the blocking accept loop never
//! stalls the async surface host, and each connection is served on its own
//! thread so a slow reply does not block later commands.

use hyprlay_core::ctl::ControlStream;
use tokio::sync::oneshot;

/// One inbound command line plus the channel to answer it on. The shell
/// parses, applies, and sends exactly one reply string.
pub struct CtlRequest {
    pub command: String,
    pub reply: oneshot::Sender<String>,
}

/// Daemon side of the control socket as an iced subscription stream.
pub fn incoming() -> impl futures_util::Stream<Item = CtlRequest> {
    use futures_util::stream;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<CtlRequest>();

    // Serve the listener on a dedicated blocking thread. A rejected bind
    // (e.g. the rare probe/bind race, or a second daemon's pipe already
    // claimed) leaves the stream empty — the daemon then runs without remote
    // control, matching the pre-existing behavior.
    std::thread::spawn(move || serve(tx));

    stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|item| (item, rx))
    })
}

fn serve(tx: tokio::sync::mpsc::UnboundedSender<CtlRequest>) {
    let path = hyprlay_core::ctl::socket_path();
    // The startup probe already unlinked any stale socket before we got
    // here. No unconditional unlink at this point: in the rare probe/bind
    // race it could delete a live daemon's fresh socket.
    let listener = match crate::platform::ipc::control::Control::listen(&path) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(event = "ctl_socket_bind_failed", error = %e, "control socket unavailable");
            return;
        }
    };
    tracing::info!(event = "ctl_socket_listening", path = %path.display(), "control socket ready");
    loop {
        let Ok(stream) = listener.accept() else {
            continue;
        };
        let tx = tx.clone();
        std::thread::spawn(move || handle(stream, tx));
    }
}

fn handle(mut stream: Box<dyn ControlStream>, tx: tokio::sync::mpsc::UnboundedSender<CtlRequest>) {
    let command = match read_control_line(&mut *stream) {
        Some(c) => c,
        None => return,
    };
    let (reply_tx, reply_rx) = oneshot::channel();
    if tx
        .send(CtlRequest {
            command,
            reply: reply_tx,
        })
        .is_ok()
        && let Ok(reply) = reply_rx.blocking_recv()
    {
        use std::io::Write;
        let _ = stream.write_all(reply.as_bytes());
        let _ = stream.write_all(b"\n");
    }
}

fn read_control_line(stream: &mut dyn std::io::Read) -> Option<String> {
    use std::io::BufRead;
    use std::io::BufReader;
    let mut line = String::new();
    let mut reader = BufReader::new(stream);
    reader.read_line(&mut line).ok().filter(|&n| n > 0)?;
    Some(line.trim().to_string())
}
