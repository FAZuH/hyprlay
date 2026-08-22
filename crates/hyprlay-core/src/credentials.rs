//! Discord own-app credential storage: `$XDG_CONFIG_HOME/hyprlay/auth.json`
//! with 0600 permissions. Lives in core because two binaries need it on
//! opposite sides of the trust boundary: the GUI writes credentials here
//! (they must never travel the ctl socket), and the daemon reads them once
//! at startup. The OAuth exchange that *uses* a stored pair stays in the
//! daemon; nothing in this module ever performs network IO.
//!
//! There is deliberately no built-in fallback identity: an absent or
//! incomplete file simply yields `None`.

use std::path::PathBuf;

/// Credentials exactly as they live in auth.json / env vars.
#[derive(Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AppCredentials {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
}

impl std::fmt::Debug for AppCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Manual impl: the secret must survive accidental `{:?}` logging.
        f.debug_struct("AppCredentials")
            .field("client_id", &self.client_id)
            .field("client_secret", &"[redacted]")
            .finish()
    }
}

impl AppCredentials {
    /// A pair is usable only when both halves are present; anything less
    /// counts as missing so half-configured setups fail closed.
    pub fn complete(&self) -> bool {
        !self.client_id.is_empty() && !self.client_secret.is_empty()
    }
}

fn auth_path() -> PathBuf {
    crate::config::config_dir().join("auth.json")
}

/// Core of [`load`] with the path injected so tests can use a tempdir
/// instead of the real config dir.
pub fn load_from(path: &std::path::Path) -> Option<AppCredentials> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(event = "auth_load_failed", error = %e, "could not read auth file");
            return None;
        }
    };
    match serde_json::from_str::<AppCredentials>(&text) {
        Ok(c) if c.complete() => Some(c),
        Ok(_) => {
            tracing::warn!(
                event = "auth_incomplete",
                "auth.json needs both client_id and client_secret; ignoring it"
            );
            None
        }
        Err(e) => {
            tracing::warn!(event = "auth_load_failed", error = %e, "corrupt auth file");
            None
        }
    }
}

/// Load the stored credentials from the default location.
pub fn load() -> Option<AppCredentials> {
    load_from(&auth_path())
}

/// Core of [`save`] with the path injected; see [`load_from`].
pub fn save_to(path: &std::path::Path, creds: &AppCredentials) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if !creds.complete() {
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        };
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let payload = serde_json::to_string_pretty(creds)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(path, payload)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

/// Persist own-app credentials with 0600 permissions, like token.json.
/// An incomplete pair removes the file (back to signed-out).
pub fn save(creds: &AppCredentials) -> std::io::Result<()> {
    save_to(&auth_path(), creds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds(id: &str, secret: &str) -> AppCredentials {
        AppCredentials {
            client_id: id.to_string(),
            client_secret: secret.to_string(),
        }
    }

    #[test]
    fn debug_formatting_never_exposes_the_client_secret() {
        // {:?} shows up in logs and panic messages; the secret must not.
        let stored = creds("visible-id", "s3cret-value");
        let text = format!("{stored:?}");
        assert!(text.contains("visible-id"));
        assert!(!text.contains("s3cret-value"));
    }

    #[test]
    fn only_complete_pairs_count_as_usable() {
        // There is deliberately no fallback identity: with only half of a
        // pair present, the credentials are treated as missing entirely.
        assert!(creds("an-id", "a-secret").complete());
        assert!(!creds("only-an-id", "").complete());
        assert!(!creds("", "only-a-secret").complete());
        assert!(!creds("", "").complete());
    }
}
