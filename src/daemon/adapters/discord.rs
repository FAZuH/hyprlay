//! Discord local RPC client over the client's unix socket (framed JSON —
//! see [`super::ipc`]), acting as the user's own registered
//! Discord application. The previous websocket bridge on port 6463 was
//! dropped deliberately: its HTTP upgrade layer validated (client_id,
//! Origin) pairs against per-application origins from the Developer
//! Portal, while the raw socket has no HTTP layer and validates nothing —
//! every properly registered application id connects without portal
//! origin configuration.
//!
//! Credentials resolve once per daemon start through `auth::detect()`.
//! With none present this loop parks after a single `credentials_missing`
//! event instead of retrying: credentials only ever appear through the GUI
//! plus a restart, which starts a fresh process.

use std::sync::Arc;
use std::time::Duration;

use futures_channel::mpsc::Sender;
use futures_util::SinkExt;
use hyprlay_core::domain::ConnectionStatus;
use serde_json::Value;
use serde_json::json;
use tracing::info;
use tracing::warn;

use super::auth::OwnAppAuth;
use super::ipc::IpcStream;
use super::token;

#[derive(Debug, Clone)]
pub enum DiscordEvent {
    Status(ConnectionStatus),
    Me(String),
    /// Voice channel name; `None` when not connected to voice.
    Channel(Option<String>),
    Participants(Vec<Participant>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Participant {
    pub id: String,
    pub name: String,
    pub avatar_hash: Option<String>,
    pub speaking: bool,
    pub self_mute: bool,
    pub self_deaf: bool,
    pub server_mute: bool,
    pub server_deaf: bool,
}

impl Participant {
    pub fn muted(&self) -> bool {
        self.self_mute || self.server_mute
    }
    pub fn deafened(&self) -> bool {
        self.self_deaf || self.server_deaf
    }
}

#[derive(Debug)]
enum SessionOutcome {
    Disconnected {
        authenticated: bool,
    },
    /// Saved token was rejected: drop it and re-run the authorize flow.
    Reauth,
}

struct Nonce(u64);
impl Nonce {
    fn next(&mut self) -> String {
        self.0 += 1;
        format!("rpc-{}", self.0)
    }
}

pub async fn run(mut sender: Sender<DiscordEvent>, auth: Option<Arc<OwnAppAuth>>) {
    let Some(auth) = auth else {
        // detect() already logged the single credentials_missing event.
        // Credentials only ever appear through the GUI plus a restart, so
        // this process can never gain them: park quietly instead of
        // hammering a socket we could not authenticate against anyway.
        info!(
            event = "rpc_parked_no_credentials",
            "overlay idle until credentials are applied and the daemon restarts"
        );
        let _ = sender
            .send(DiscordEvent::Status(ConnectionStatus::Disconnected))
            .await;
        // Diverge by awaiting a future that never resolves: the task stays
        // asleep (no polling churn) for the rest of the process lifetime.
        loop {
            std::future::pending::<()>().await;
        }
    };
    let mut backoff = Duration::from_secs(1);
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        let _ = sender
            .send(DiscordEvent::Status(ConnectionStatus::Connecting))
            .await;
        match session(&mut sender, &auth).await {
            SessionOutcome::Disconnected { authenticated } => {
                let _ = sender
                    .send(DiscordEvent::Status(ConnectionStatus::Disconnected))
                    .await;
                let _ = sender.send(DiscordEvent::Participants(Vec::new())).await;
                let _ = sender.send(DiscordEvent::Channel(None)).await;
                if authenticated {
                    backoff = Duration::from_secs(1);
                }
            }
            SessionOutcome::Reauth => {
                token::remove();
                backoff = Duration::from_secs(1);
                continue;
            }
        }
        info!(
            event = "rpc_reconnect_scheduled",
            attempt,
            backoff_ms = backoff.as_millis() as u64,
            "session ended, retrying"
        );
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

/// One IPC connection lifetime. The `rpc_session` span accumulates
/// everything we know about how it went and closes with a single outcome
/// event — the wide event for this unit of work.
#[tracing::instrument(name = "rpc_session", skip(sender, auth))]
async fn session(sender: &mut Sender<DiscordEvent>, auth: &Arc<OwnAppAuth>) -> SessionOutcome {
    let span = tracing::Span::current();

    let mut conn = match IpcStream::connect().await {
        Ok(conn) => conn,
        Err(e) => {
            span.record("outcome", "connect_failed");
            warn!(event = "rpc_connect_failed", error = %e, "cannot reach discord rpc");
            return SessionOutcome::Disconnected {
                authenticated: false,
            };
        }
    };
    if let Err(e) = conn.handshake(&auth.client_id).await {
        span.record("outcome", "connect_failed");
        warn!(event = "rpc_connect_failed", error = %e, "could not send handshake");
        return SessionOutcome::Disconnected {
            authenticated: false,
        };
    }
    let mut nonce = Nonce(0);

    // The RPC server silently drops any frame sent before its READY dispatch,
    // so the first command must wait for it.
    let ready = loop {
        match tokio::time::timeout(Duration::from_secs(10), conn.next_frame()).await {
            Ok(Some(v)) if v["evt"].as_str() == Some("READY") => break true,
            Ok(Some(_)) => {}
            _ => break false,
        }
    };
    if !ready {
        span.record("outcome", "no_ready_dispatch");
        warn!(event = "rpc_no_ready", "server never sent READY");
        return SessionOutcome::Disconnected {
            authenticated: false,
        };
    }

    // A cached token only authenticates if it was issued to the same
    // application we are connecting as; token.rs discards foreign ones.
    let token = token::load(&auth.client_id);
    if let Some(token) = token {
        let _ = sender
            .send(DiscordEvent::Status(ConnectionStatus::Authenticating))
            .await;
        // Token values never enter log fields.
        let _ = conn
            .send(&request(
                "AUTHENTICATE",
                json!({ "access_token": token }),
                None,
                &nonce.next(),
            ))
            .await;
    } else {
        let _ = sender
            .send(DiscordEvent::Status(ConnectionStatus::Authorize))
            .await;
        let _ = conn
            .send(&request(
                "AUTHORIZE",
                json!({
                    "client_id": auth.client_id.as_str(),
                    "scopes": ["identify", "rpc"],
                    // Auto-approve without the modal when the app was
                    // authorized before; first-timers see the modal.
                    "prompt": "none",
                }),
                None,
                &nonce.next(),
            ))
            .await;
    }

    let mut users: Vec<Participant> = Vec::new();
    let mut channel_id: Option<String> = None;
    let mut authenticated = false;

    while let Some(v) = conn.next_frame().await {
        // Events carry a non-null `evt`; responses echo `cmd` + `nonce`.
        let evt = v["evt"].as_str();

        match evt {
            Some("ERROR") => {
                let code = v["data"]["code"].as_i64().unwrap_or(0);
                let msg = v["data"]["message"].as_str().unwrap_or("unknown error");
                warn!(event = "rpc_command_error", code, message = %sanitize(msg), "discord rejected a command");
                // 4007 INVALID_CLIENT_ID, 4008 OAUTH2_ERROR, 4009 INVALID_TOKEN, 4010 INVALID_USER
                if matches!(code, 4007..=4010) {
                    span.record("rpc_error_code", code);
                    if authenticated {
                        return SessionOutcome::Reauth;
                    }
                    // Token was rejected before login: drop it and authorize.
                    let _ = conn
                        .send(&request(
                            "AUTHORIZE",
                            json!({
                                "client_id": auth.client_id.as_str(),
                                "scopes": ["identify", "rpc"],
                                "prompt": "none",
                            }),
                            None,
                            &nonce.next(),
                        ))
                        .await;
                } else if !authenticated {
                    // The AUTHORIZE attempt itself failed hard (e.g. 5000
                    // when the application lacks its registered desktop
                    // redirect URI). No command can repair this connection,
                    // so end it: run() backs off and sends a fresh AUTHORIZE,
                    // which self-heals once the portal config is fixed.
                    span.record("outcome", "authorize_failed");
                    return SessionOutcome::Disconnected {
                        authenticated: false,
                    };
                }
            }
            Some("VOICE_CHANNEL_SELECT") => {
                let new_channel = v["data"]["channel_id"].as_str().map(str::to_string);
                info!(
                    event = "voice_channel_select",
                    switched = new_channel.is_some(),
                    "client joined/left a voice channel"
                );
                if new_channel != channel_id {
                    switch_channel(
                        &mut conn,
                        &mut users,
                        channel_id.as_deref(),
                        new_channel.as_deref(),
                        sender,
                    )
                    .await;
                    channel_id = new_channel;
                }
            }
            Some("VOICE_STATE_CREATE") | Some("VOICE_STATE_UPDATE") => {
                if let Some(p) = parse_participant(&v["data"]) {
                    match users.iter_mut().find(|u| u.id == p.id) {
                        Some(existing) => {
                            let speaking = existing.speaking;
                            *existing = p;
                            existing.speaking = speaking;
                        }
                        None => users.push(p),
                    }
                    let _ = sender.send(DiscordEvent::Participants(users.clone())).await;
                }
            }
            Some("VOICE_STATE_DELETE") => {
                if let Some(id) = v["data"]["user"]["id"].as_str() {
                    users.retain(|u| u.id != id);
                    let _ = sender.send(DiscordEvent::Participants(users.clone())).await;
                }
            }
            Some("SPEAKING_START") | Some("SPEAKING_STOP") => {
                let speaking = evt == Some("SPEAKING_START");
                if let Some(id) = v["data"]["user_id"].as_str() {
                    if let Some(u) = users.iter_mut().find(|u| u.id == id) {
                        u.speaking = speaking;
                    }
                    let _ = sender.send(DiscordEvent::Participants(users.clone())).await;
                }
            }
            Some(_) => {}
            None => match v["cmd"].as_str() {
                Some("AUTHORIZE") => {
                    let Some(code) = v["data"]["code"].as_str() else {
                        continue;
                    };
                    let _ = sender
                        .send(DiscordEvent::Status(ConnectionStatus::ExchangingToken))
                        .await;
                    let code = code.to_string();
                    // The code is a short-lived credential: it goes to the
                    // token endpoint and nowhere else (never logged). The
                    // blocking task owns an Arc clone so the session can
                    // keep using the credentials afterwards.
                    let exchange_auth = auth.clone();
                    let exchanged =
                        tokio::task::spawn_blocking(move || exchange_auth.exchange(&code)).await;
                    let token = match exchanged {
                        Ok(Ok(token)) => token,
                        Ok(Err(err)) => {
                            span.record("outcome", "token_exchange_failed");
                            warn!(
                                event = "token_exchange_failed",
                                error = %err,
                                "token exchange rejected the authorize code"
                            );
                            return SessionOutcome::Disconnected {
                                authenticated: false,
                            };
                        }
                        Err(err) => {
                            span.record("outcome", "token_exchange_failed");
                            warn!(
                                event = "token_exchange_failed",
                                error = %err,
                                "token exchange task failed"
                            );
                            return SessionOutcome::Disconnected {
                                authenticated: false,
                            };
                        }
                    };
                    token::save(&token, &auth.client_id);
                    let _ = conn
                        .send(&request(
                            "AUTHENTICATE",
                            json!({ "access_token": token }),
                            None,
                            &nonce.next(),
                        ))
                        .await;
                }
                Some("AUTHENTICATE") => {
                    authenticated = true;
                    span.record("authenticated", true);
                    span.record(
                        "user.id",
                        v["data"]["user"]["id"].as_str().unwrap_or("unknown"),
                    );
                    info!(event = "rpc_authenticated", "logged in to discord rpc");
                    let me = v["data"]["user"]["id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    let _ = sender.send(DiscordEvent::Me(me)).await;
                    let _ = sender
                        .send(DiscordEvent::Status(ConnectionStatus::Connected))
                        .await;
                    let _ = conn
                        .send(&request(
                            "SUBSCRIBE",
                            json!({}),
                            Some("VOICE_CHANNEL_SELECT"),
                            &nonce.next(),
                        ))
                        .await;
                    let _ = conn
                        .send(&request(
                            "GET_SELECTED_VOICE_CHANNEL",
                            json!({}),
                            None,
                            &nonce.next(),
                        ))
                        .await;
                }
                Some("GET_SELECTED_VOICE_CHANNEL") => {
                    let data = &v["data"];
                    if data.is_null() {
                        users.clear();
                        channel_id = None;
                        let _ = sender.send(DiscordEvent::Channel(None)).await;
                        let _ = sender.send(DiscordEvent::Participants(Vec::new())).await;
                    } else {
                        let new_id = data["id"].as_str().map(str::to_string);
                        if new_id != channel_id {
                            switch_channel(
                                &mut conn,
                                &mut users,
                                channel_id.as_deref(),
                                new_id.as_deref(),
                                sender,
                            )
                            .await;
                            channel_id = new_id;
                        }
                        users = data["voice_states"]
                            .as_array()
                            .map(|a| a.iter().filter_map(parse_participant).collect())
                            .unwrap_or_default();
                        info!(
                            event = "voice_channel_joined",
                            channel = %data["name"].as_str().unwrap_or("?"),
                            participants = users.len(),
                            "selected voice channel"
                        );
                        // A closed channel means the app is shutting down.
                        let _ = sender
                            .send(DiscordEvent::Channel(
                                data["name"].as_str().map(str::to_string),
                            ))
                            .await;
                        let _ = sender.send(DiscordEvent::Participants(users.clone())).await;
                    }
                }
                _ => {}
            },
        }
    }

    span.record("outcome", "closed");
    info!(event = "rpc_session_ended", "ipc connection closed");
    SessionOutcome::Disconnected { authenticated }
}

const CHANNEL_EVENTS: [&str; 5] = [
    "VOICE_STATE_CREATE",
    "VOICE_STATE_UPDATE",
    "VOICE_STATE_DELETE",
    "SPEAKING_START",
    "SPEAKING_STOP",
];

/// Unsubscribe from `old`, reset participants, subscribe to `new`.
async fn switch_channel(
    conn: &mut IpcStream,
    users: &mut Vec<Participant>,
    old: Option<&str>,
    new: Option<&str>,
    sender: &mut Sender<DiscordEvent>,
) {
    if let Some(old) = old {
        for evt in CHANNEL_EVENTS {
            let _ = conn
                .send(&request(
                    "UNSUBSCRIBE",
                    json!({ "channel_id": old }),
                    Some(evt),
                    &format!("rpc-unsub-{evt}"),
                ))
                .await;
        }
    }
    users.clear();
    let _ = sender.send(DiscordEvent::Participants(Vec::new())).await;
    if let Some(new) = new {
        for evt in CHANNEL_EVENTS {
            let _ = conn
                .send(&request(
                    "SUBSCRIBE",
                    json!({ "channel_id": new }),
                    Some(evt),
                    &format!("rpc-sub-{evt}"),
                ))
                .await;
        }
    } else {
        let _ = sender.send(DiscordEvent::Channel(None)).await;
    }
}

fn request(cmd: &str, args: Value, evt: Option<&str>, nonce: &str) -> Value {
    let mut v = json!({ "cmd": cmd, "args": args, "nonce": nonce });
    if let Some(evt) = evt {
        v["evt"] = json!(evt);
    }
    v
}

pub(crate) fn parse_participant(v: &Value) -> Option<Participant> {
    let user = &v["user"];
    let id = user["id"].as_str()?.to_string();
    let username = user["username"].as_str().unwrap_or("unknown");
    let nick = v["nick"].as_str().filter(|n| !n.is_empty());
    let vs = &v["voice_state"];
    Some(Participant {
        id,
        name: nick.unwrap_or(username).to_string(),
        avatar_hash: user["avatar"].as_str().map(str::to_string),
        speaking: false,
        self_mute: vs["self_mute"].as_bool().unwrap_or(false),
        self_deaf: vs["self_deaf"].as_bool().unwrap_or(false),
        server_mute: vs["mute"].as_bool().unwrap_or(false),
        server_deaf: vs["deaf"].as_bool().unwrap_or(false),
    })
}

/// Discord error messages are free text from the wire: strip control
/// characters and cap length before they enter a log field.
fn sanitize(s: &str) -> String {
    let mut out: String = s.chars().filter(|c| !c.is_control()).take(200).collect();
    if out.len() == 200 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice_state(v: &str) -> Value {
        serde_json::from_str(v).unwrap()
    }

    #[test]
    fn parse_participant_prefers_non_empty_nick_over_username() {
        let v = voice_state(
            r#"{"nick": "Fazuh", "user": {"id": "238uwenrfi", "username": "old_name", "avatar": "a1b2c3"},
                "voice_state": {"self_mute": false}}"#,
        );
        let p = parse_participant(&v).unwrap();
        assert_eq!(p.name, "Fazuh");
        assert_eq!(p.id, "238uwenrfi");
        assert_eq!(p.avatar_hash.as_deref(), Some("a1b2c3"));
    }

    #[test]
    fn parse_participant_falls_back_to_username_when_nick_empty() {
        let v = voice_state(
            r#"{"nick": "", "user": {"id": "123456789012345678", "username": "fazuh"}}"#,
        );
        let p = parse_participant(&v).unwrap();
        assert_eq!(p.name, "fazuh");
    }

    #[test]
    fn parse_participant_returns_none_without_user_id() {
        let v = voice_state(r#"{"user": {"username": "ghost"}, "voice_state": {}}"#);
        assert!(parse_participant(&v).is_none());
    }

    #[test]
    fn parse_participant_defaults_all_flags_false_when_voice_state_missing() {
        let v = voice_state(r#"{"user": {"id": "42", "username" : "plain"}}"#);
        let p = parse_participant(&v).unwrap();
        assert!(!p.speaking && !p.self_mute && !p.self_deaf && !p.server_mute && !p.server_deaf);
        assert_eq!(p.avatar_hash, None);
    }

    #[test]
    fn parse_participant_reads_server_and_self_flags_distinctly() {
        let v = voice_state(
            r#"{"user": {"id": "7", "username": "modded"},
                "voice_state": {"self_mute": false, "self_deaf": false, "mute": true, "deaf": false}}"#,
        );
        let p = parse_participant(&v).unwrap();
        assert!(p.muted());
        assert!(!p.deafened());
    }

    #[test]
    fn request_omits_evt_field_when_none() {
        let v = request("PING", json!({}), None, "n-1");
        assert!(v.get("evt").is_none());
        assert_eq!(v["nonce"], "n-1");
    }

    #[test]
    fn request_includes_evt_field_when_given() {
        let v = request(
            "SUBSCRIBE",
            json!({"channel_id": "99"}),
            Some("SPEAKING_START"),
            "n-2",
        );
        assert_eq!(v["evt"], "SPEAKING_START");
        assert_eq!(v["cmd"], "SUBSCRIBE");
    }

    #[test]
    fn sanitize_strips_control_characters_and_caps_length() {
        assert_eq!(sanitize("clean"), "clean");
        assert_eq!(sanitize("bad\u{0007}noise"), "badnoise");
        let long = "x".repeat(500);
        assert_eq!(sanitize(&long).chars().count(), 201);
    }
}
