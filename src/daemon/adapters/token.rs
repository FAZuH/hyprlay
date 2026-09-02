//! OAuth token persistence: `$XDG_CONFIG_HOME/hyprlay/token.json`.
//!
//! Tokens are stored bound to the application id they were issued for.
//! Discord rejects cross-application tokens with 4007 ("Application does
//! not match"), so on load a cached token whose issuer differs from the
//! expected backend id is treated as stale and deleted instead of being
//! retried on every boot.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use serde_json::Value;
use serde_json::json;

fn path() -> PathBuf {
    hyprlay_core::config::config_dir().join("token.json")
}

/// Load the cached access token if it was issued to `expected_client_id`.
pub fn load(expected_client_id: &str) -> Option<String> {
    load_from(&path(), expected_client_id)
}

// Core of load with the path injected so tests can use a tempdir instead of
// the real config dir.
#[doc(hidden)]
pub fn load_from(path: &Path, expected_client_id: &str) -> Option<String> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        // No token yet is the normal first-run state, not an error.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(event = "token_load_failed", error = %e, "could not read token file");
            return None;
        }
    };
    let v: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(event = "token_load_failed", error = %e, "corrupt token file");
            return None;
        }
    };
    let token = v["access_token"].as_str().map(str::to_string)?;
    match v["client_id"].as_str() {
        // A file without an issuer predates app binding and cannot be
        // trusted for any application: discard it outright.
        None => {
            tracing::info!(
                event = "token_without_app_binding_discarded",
                "cached token carries no application binding; discarding"
            );
            remove_at(path);
            None
        }
        Some(saved_id) if saved_id == expected_client_id => Some(token),
        // A token minted for another application can never authenticate
        // here: drop it so the next session re-authorizes fresh.
        Some(_) => {
            tracing::info!(
                event = "token_from_other_app_discarded",
                "cached token belongs to another discord application"
            );
            remove_at(path);
            None
        }
    }
}

/// Persist the access token together with the application that owns it.
pub fn save(token: &str, client_id: &str) {
    save_to(&path(), token, client_id);
}

// Core of save with the path injected; see load_from.
#[doc(hidden)]
pub fn save_to(path: &Path, token: &str, client_id: &str) {
    let write = || -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let payload = serde_json::to_string(&json!({
            "access_token": token,
            "client_id": client_id,
        }))
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        fs::write(path, payload)
    };
    if let Err(e) = write() {
        tracing::warn!(event = "token_save_failed", error = %e, "could not write token");
        return;
    }
    if let Err(e) = hyprlay_core::platform::secure_perms(path) {
        tracing::warn!(
            event = "token_chmod_failed",
            error = %e,
            "could not restrict token file permissions"
        );
    }
}

/// Drop a rejected token so the next session re-runs the authorize flow.
pub fn remove() {
    remove_at(&path());
}

// Core of remove with the path injected; see load_from.
fn remove_at(path: &Path) {
    // Missing file is fine — that is already the logged-out state.
    if let Err(e) = fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(event = "token_remove_failed", error = %e, "could not delete token");
    }
}
