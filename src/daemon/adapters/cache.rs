//! Disk caches under `$XDG_CACHE_HOME/hyprlay/` so a daemon restart
//! (e.g. a monitor switch) renders the overlay instantly instead of showing
//! "waiting for discord":
//!
//! - `roster.json`: last-known channel + participants (written only while
//!   connected, deduplicated by roster signature so speaking toggles don't
//!   touch the disk)
//! - `avatars/<user>-<hash>.png`: avatar images keyed by content hash

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::OnceLock;

use serde::Deserialize;
use serde::Serialize;

use super::discord::Participant;

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .map(|d| d.join("hyprlay"))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Roster {
    pub channel: Option<String>,
    pub me_id: Option<String>,
    pub users: Vec<Participant>,
}

/// Cheap identity of a roster for write dedup — the serde output of every
/// participant, so the signature covers exactly the fields the cache file
/// persists and can never drift away from the file format. The live-only
/// speaking flag is skipped by serde and therefore never reaches either.
/// A silent `unwrap_or_default()` fallback is deliberately not used: it
/// would turn every failing entry into the same empty string, collapsing
/// distinct users into equal signatures and silently skipping writes.
fn roster_signature(users: &[Participant]) -> String {
    users
        .iter()
        .map(|u| {
            serde_json::to_string(u)
                .expect("roster entries are plain strings and bools; they always serialize")
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn last_signature() -> &'static Mutex<String> {
    static LAST: OnceLock<Mutex<String>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(String::new()))
}

/// Persist the roster if it changed since the last write.
pub fn save_roster(channel: Option<&str>, me_id: Option<&str>, users: &[Participant]) {
    let sig = format!("{:?}|{:?}|{}", channel, me_id, roster_signature(users));
    {
        let mut last = last_signature().lock().unwrap();
        if *last == sig {
            return;
        }
        *last = sig;
    }
    let roster = Roster {
        channel: channel.map(str::to_string),
        me_id: me_id.map(str::to_string),
        users: users.to_vec(),
    };
    let dir = cache_dir();
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        let payload = serde_json::to_string(&roster)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(dir.join("roster.json"), payload)
    };
    if let Err(e) = write() {
        tracing::warn!(event = "roster_save_failed", error = %e, "could not persist roster cache");
    }
}

pub fn load_roster() -> Option<Roster> {
    let text = match std::fs::read_to_string(cache_dir().join("roster.json")) {
        Ok(t) => t,
        // First run after install has no cache yet — that is normal.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(event = "roster_load_failed", error = %e, "could not read roster cache");
            return None;
        }
    };
    match serde_json::from_str(&text) {
        Ok(roster) => Some(roster),
        Err(e) => {
            tracing::warn!(event = "roster_load_failed", error = %e, "corrupt roster cache");
            None
        }
    }
}

pub fn avatar_path(user_id: &str, hash: &str) -> PathBuf {
    cache_dir()
        .join("avatars")
        .join(format!("{user_id}-{hash}.png"))
}

/// A cache miss is normal (avatar not fetched yet) and stays silent.
pub fn load_avatar(user_id: &str, hash: &str) -> Option<Vec<u8>> {
    std::fs::read(avatar_path(user_id, hash)).ok()
}

pub fn store_avatar(user_id: &str, hash: &str, bytes: &[u8]) {
    let path = avatar_path(user_id, hash);
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, bytes)
    };
    if let Err(e) = write() {
        tracing::warn!(event = "avatar_cache_failed", error = %e, user.id = user_id, "could not cache avatar");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn participant(id: &str, name: &str, hash: Option<&str>, mutes: (bool, bool)) -> Participant {
        Participant {
            id: id.to_string(),
            name: name.to_string(),
            avatar_hash: hash.map(str::to_string),
            speaking: true, // must be dropped by serde
            self_mute: mutes.0,
            self_deaf: mutes.1,
            server_mute: false,
            server_deaf: false,
        }
    }

    /// The exact bytes the cache wrote before the roster type was unified:
    /// the seven persisted user fields in declaration order, no `speaking`
    /// key anywhere. Pinned so existing caches load with no migration.
    const LEGACY_ROSTER_JSON: &str = r#"{"channel":"General","me_id":"238492734982739483","users":[{"id":"238492734982739483","name":"fazuh","avatar_hash":"a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6","self_mute":true,"self_deaf":false,"server_mute":false,"server_deaf":false},{"id":"1079875395007417102","name":"quiet_guest","avatar_hash":null,"self_mute":false,"self_deaf":false,"server_mute":true,"server_deaf":false}]}"#;

    #[test]
    fn roster_cache_from_the_previous_format_still_loads() {
        let roster: Roster = serde_json::from_str(LEGACY_ROSTER_JSON).unwrap();
        assert_eq!(roster.channel.as_deref(), Some("General"));
        assert_eq!(roster.me_id.as_deref(), Some("238492734982739483"));
        assert_eq!(roster.users.len(), 2);
        assert_eq!(roster.users[0].id, "238492734982739483");
        assert_eq!(roster.users[0].name, "fazuh");
        assert_eq!(
            roster.users[0].avatar_hash.as_deref(),
            Some("a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6")
        );
        assert!(roster.users[0].self_mute);
        assert!(!roster.users[0].self_deaf);
        assert!(!roster.users[0].server_mute);
        assert!(!roster.users[0].server_deaf);
        assert_eq!(roster.users[1].id, "1079875395007417102");
        assert_eq!(roster.users[1].name, "quiet_guest");
        assert_eq!(roster.users[1].avatar_hash, None);
        assert!(roster.users[1].server_mute);
        // The live-only speaking flag never survives a load: serde fills
        // it with false, same as the previous cache format's load path.
        assert!(!roster.users[0].speaking);
        assert!(!roster.users[1].speaking);
    }

    #[test]
    fn roster_cache_writes_the_same_bytes_as_the_previous_format() {
        let mut server_muted =
            participant("1079875395007417102", "quiet_guest", None, (false, false));
        server_muted.server_mute = true;
        // Both participants carry speaking=true on purpose: the flag is
        // skipped by serde, so the bytes stay legacy-identical.
        let roster = Roster {
            channel: Some("General".to_string()),
            me_id: Some("238492734982739483".to_string()),
            users: vec![
                participant(
                    "238492734982739483",
                    "fazuh",
                    Some("a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6"),
                    (true, false),
                ),
                server_muted,
            ],
        };
        assert_eq!(serde_json::to_string(&roster).unwrap(), LEGACY_ROSTER_JSON);
    }

    #[test]
    fn roster_signature_changes_when_any_persisted_field_changes() {
        let base = participant(
            "238492734982739483",
            "fazuh",
            Some("a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6"),
            (true, false),
        );
        let sig = |u: &Participant| roster_signature(std::slice::from_ref(u));
        let base_sig = sig(&base);

        let mut changed = base.clone();
        changed.id = "1079875395007417102".to_string();
        assert_ne!(sig(&changed), base_sig, "id must reach the signature");

        let mut changed = base.clone();
        changed.name = "renamed".to_string();
        assert_ne!(sig(&changed), base_sig, "name must reach the signature");

        let mut changed = base.clone();
        changed.avatar_hash = None;
        assert_ne!(
            sig(&changed),
            base_sig,
            "avatar_hash must reach the signature"
        );

        let mut changed = base.clone();
        changed.self_mute = false;
        assert_ne!(
            sig(&changed),
            base_sig,
            "self_mute must reach the signature"
        );

        let mut changed = base.clone();
        changed.self_deaf = true;
        assert_ne!(
            sig(&changed),
            base_sig,
            "self_deaf must reach the signature"
        );

        let mut changed = base.clone();
        changed.server_mute = true;
        assert_ne!(
            sig(&changed),
            base_sig,
            "server_mute must reach the signature"
        );

        let mut changed = base.clone();
        changed.server_deaf = true;
        assert_ne!(
            sig(&changed),
            base_sig,
            "server_deaf must reach the signature"
        );

        // Parity with the old hand-written signature: it enumerated only
        // the persisted fields, so a speaking flip must stay invisible.
        let mut changed = base.clone();
        changed.speaking = !changed.speaking;
        assert_eq!(
            sig(&changed),
            base_sig,
            "speaking must not reach the signature"
        );
    }

    #[test]
    fn participant_serde_roundtrip_drops_speaking_flag() {
        let p = participant("42", "fazuh", Some("abc"), (true, false));
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("speaking"));
        let restored: Participant = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.id, "42");
        assert_eq!(restored.name, "fazuh");
        assert_eq!(restored.avatar_hash.as_deref(), Some("abc"));
        assert!(restored.self_mute);
        assert!(!restored.speaking);
    }

    #[test]
    fn roster_signature_ignores_speaking_but_not_mutes() {
        let mut a = participant("1", "a", None, (false, false));
        let b = participant("1", "a", None, (false, false));
        // Same user twice → equal signatures.
        assert_eq!(
            roster_signature(std::slice::from_ref(&a)),
            roster_signature(std::slice::from_ref(&b))
        );
        a.self_mute = true;
        assert_ne!(
            roster_signature(std::slice::from_ref(&a)),
            roster_signature(std::slice::from_ref(&b))
        );
    }

    #[test]
    fn avatar_path_is_keyed_by_user_and_hash() {
        let p = avatar_path("123", "deadbeef");
        assert!(p.to_string_lossy().contains("123-deadbeef.png"));
        assert_ne!(avatar_path("123", "deadbeef"), avatar_path("123", "other"));
    }
}
