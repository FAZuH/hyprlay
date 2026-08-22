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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct CachedUser {
    pub id: String,
    pub name: String,
    pub avatar_hash: Option<String>,
    pub self_mute: bool,
    pub self_deaf: bool,
    pub server_mute: bool,
    pub server_deaf: bool,
}

impl From<&Participant> for CachedUser {
    fn from(p: &Participant) -> Self {
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            avatar_hash: p.avatar_hash.clone(),
            self_mute: p.self_mute,
            self_deaf: p.self_deaf,
            server_mute: p.server_mute,
            server_deaf: p.server_deaf,
        }
    }
}

impl From<CachedUser> for Participant {
    fn from(u: CachedUser) -> Self {
        // Speaking state is live-only: it would be stale on load.
        Participant {
            id: u.id,
            name: u.name,
            avatar_hash: u.avatar_hash,
            speaking: false,
            self_mute: u.self_mute,
            self_deaf: u.self_deaf,
            server_mute: u.server_mute,
            server_deaf: u.server_deaf,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Roster {
    pub channel: Option<String>,
    pub me_id: Option<String>,
    pub users: Vec<CachedUser>,
}

/// Cheap identity of a roster for write dedup — everything except the
/// per-event speaking flag.
fn roster_signature(users: &[CachedUser]) -> String {
    users
        .iter()
        .map(|u| {
            format!(
                "{}|{}|{:?}|{}{}{}{}",
                u.id,
                u.name,
                u.avatar_hash,
                u.self_mute as u8,
                u.self_deaf as u8,
                u.server_mute as u8,
                u.server_deaf as u8
            )
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
    let cached: Vec<CachedUser> = users.iter().map(CachedUser::from).collect();
    let sig = format!("{:?}|{:?}|{}", channel, me_id, roster_signature(&cached));
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
        users: cached,
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
            speaking: true, // must be dropped by the conversion
            self_mute: mutes.0,
            self_deaf: mutes.1,
            server_mute: false,
            server_deaf: false,
        }
    }

    #[test]
    fn cached_user_roundtrip_drops_speaking_flag() {
        let p = participant("42", "fazuh", Some("abc"), (true, false));
        let back = CachedUser::from(&p);
        let restored: Participant = back.into();
        assert_eq!(restored.id, "42");
        assert_eq!(restored.name, "fazuh");
        assert_eq!(restored.avatar_hash.as_deref(), Some("abc"));
        assert!(restored.self_mute);
        assert!(!restored.speaking);
    }

    #[test]
    fn roster_signature_ignores_speaking_but_not_mutes() {
        let mut a = CachedUser::from(&participant("1", "a", None, (false, false)));
        let b = CachedUser::from(&participant("1", "a", None, (false, false)));
        // Same user twice → equal signatures.
        assert_eq!(
            roster_signature(&[a.clone()]),
            roster_signature(std::slice::from_ref(&b))
        );
        a.self_mute = true;
        assert_ne!(roster_signature(&[a]), roster_signature(&[b]));
    }

    #[test]
    fn avatar_path_is_keyed_by_user_and_hash() {
        let p = avatar_path("123", "deadbeef");
        assert!(p.to_string_lossy().contains("123-deadbeef.png"));
        assert_ne!(avatar_path("123", "deadbeef"), avatar_path("123", "other"));
    }
}
