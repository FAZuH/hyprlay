//! Own-application Discord credentials and the OAuth token exchange.
//!
//! Owning the overlay means owning the application: the client id and
//! secret come from environment variables or auth.json (mode 0600) — never
//! from config.toml or the ctl socket, so `dump` and friends can never leak
//! them. Storage of auth.json lives in `hyprlay-core::credentials` because
//! the settings GUI writes it directly; this module owns the daemon-side
//! half: resolving the process's credential pair and exchanging authorize
//! codes for tokens. There is deliberately no built-in fallback identity:
//! without a complete pair detect() reports a missing-credentials error
//! state and the RPC client stays offline instead of pretending to work.

use std::fmt;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use hyprlay_core::credentials::AppCredentials;
use serde_json::Value;

const DISCORD_TOKEN_URL: &str = "https://discord.com/api/oauth2/token";

/// Desktop redirect URI an own application must have registered in its
/// Developer Portal OAuth2 settings for the local-RPC authorize flow to
/// work at all (the Social SDK documents exactly this value), and which
/// the token endpoint requires to be repeated in every authorization_code
/// exchange — a mismatch is rejected with "Missing redirect_uri".
pub const OWN_APP_REDIRECT_URI: &str = "http://127.0.0.1/callback";

/// How long to wait for a token endpoint before giving up.
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(15);

/// Why a code exchange failed. Display strings are safe to log: they never
/// contain the code, the token, or the client secret.
#[derive(Debug)]
pub enum ExchangeError {
    RequestFailed(&'static str),
}

impl fmt::Display for ExchangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestFailed(why) => write!(f, "token request failed: {why}"),
        }
    }
}

impl std::error::Error for ExchangeError {}

/// The stored identity of the user's own Discord application: what the
/// local-RPC handshake and AUTHORIZE payloads present, and what signs the
/// token exchange. Only detect() constructs instances, so one always
/// carries a complete credential pair.
#[derive(Clone)]
pub struct OwnAppAuth {
    /// Application id sent in the handshake and AUTHORIZE payloads.
    pub client_id: String,
    // Kept private so it can never be formatted by accident.
    client_secret: String,
}

impl fmt::Debug for OwnAppAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Manual impl: derive(Debug) would print the client secret into any
        // log line or panic message that ever touches `{:?}`.
        f.debug_struct("OwnAppAuth")
            .field("client_id", &self.client_id)
            .field("client_secret", &"[redacted]")
            .finish()
    }
}

impl OwnAppAuth {
    fn new(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
        }
    }

    // Pure helper so the wire shape is unit-testable without network.
    fn basic_auth_header(&self) -> String {
        format!(
            "Basic {}",
            BASE64.encode(format!("{}:{}", self.client_id, self.client_secret)),
        )
    }

    // Pure helper so the exchange form is unit-testable without network:
    // grant_type, the authorize code, and the portal-registered redirect
    // (see OWN_APP_REDIRECT_URI) must all be present or Discord rejects it.
    fn exchange_body(code: &str) -> [(&'static str, String); 3] {
        [
            ("grant_type", "authorization_code".to_string()),
            ("code", code.to_string()),
            ("redirect_uri", OWN_APP_REDIRECT_URI.to_string()),
        ]
    }

    /// Exchange an IPC authorize code for an access token. Blocking; run it
    /// off the async runtime.
    pub fn exchange(&self, code: &str) -> Result<String, ExchangeError> {
        // Discord only accepts form-urlencoded bodies here; send_form sets
        // the content type and percent-encodes every pair.
        let body = Self::exchange_body(code);
        let form: Vec<(&str, &str)> = body.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let resp = ureq::post(DISCORD_TOKEN_URL)
            .timeout(EXCHANGE_TIMEOUT)
            .set("Authorization", &self.basic_auth_header())
            .send_form(&form)
            .map_err(|_| ExchangeError::RequestFailed("discord rejected the request"))?;
        let v: Value = resp
            .into_json()
            .map_err(|_| ExchangeError::RequestFailed("discord sent invalid JSON"))?;
        v["access_token"]
            .as_str()
            .map(str::to_string)
            .ok_or(ExchangeError::RequestFailed("discord reply had no token"))
    }
}

// Core of detect() with both sources injected; pure so tests can exercise
// precedence without touching process env (parallel test threads share it).
fn detect_with(
    env_creds: Option<AppCredentials>,
    file_creds: Option<AppCredentials>,
) -> Option<OwnAppAuth> {
    env_creds
        .filter(|c| c.complete())
        .or_else(|| file_creds.filter(|c| c.complete()))
        .map(|c| OwnAppAuth::new(c.client_id, c.client_secret))
}

/// Resolve this process's credentials: environment variables win over
/// auth.json (loaded via [`hyprlay_core::credentials`]), and anything
/// incomplete counts as missing. `None` is the logged-once error state —
/// credentials only ever appear through the GUI plus a daemon restart,
/// never mid-process.
pub fn detect() -> Option<OwnAppAuth> {
    let env_creds = match (
        std::env::var("DISCORD_CLIENT_ID"),
        std::env::var("DISCORD_CLIENT_SECRET"),
    ) {
        (Ok(id), Ok(secret)) => Some(AppCredentials {
            client_id: id,
            client_secret: secret,
        }),
        _ => None,
    };
    let auth = detect_with(env_creds, hyprlay_core::credentials::load());
    if auth.is_some() {
        tracing::info!(
            event = "auth_backend",
            backend = "own-app",
            "own application credentials resolved"
        );
    } else {
        tracing::warn!(
            event = "credentials_missing",
            "no usable Discord credentials: set DISCORD_CLIENT_ID and DISCORD_CLIENT_SECRET or apply them in the GUI Connection section; the overlay stays offline until then"
        );
    }
    auth
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
    fn own_app_basic_auth_header_encodes_id_and_secret_as_base64() {
        // The expected value is computed here from first principles, not via
        // the code under test, so encoding regressions cannot hide.
        let expected = format!("Basic {}", BASE64.encode("123456789012345678:s3cret-value"));
        let backend = OwnAppAuth::new("123456789012345678", "s3cret-value");
        assert_eq!(backend.basic_auth_header(), expected);
    }

    #[test]
    fn own_app_exchange_body_carries_grant_type_code_and_redirect_uri() {
        assert_eq!(
            OwnAppAuth::exchange_body("auth-code-123"),
            [
                ("grant_type", "authorization_code".to_string()),
                ("code", "auth-code-123".to_string()),
                ("redirect_uri", OWN_APP_REDIRECT_URI.to_string()),
            ]
        );
    }

    #[test]
    fn debug_formatting_never_exposes_the_client_secret() {
        // {:?} shows up in logs and panic messages; the secret must not —
        // neither in stored credentials nor in the resolved backend.
        let creds = creds("visible-id", "s3cret-value");
        let text = format!("{creds:?}");
        assert!(text.contains("visible-id"));
        assert!(!text.contains("s3cret-value"));

        let backend = OwnAppAuth::new("visible-id", "s3cret-value");
        assert!(!format!("{backend:?}").contains("s3cret-value"));
    }

    #[test]
    fn detect_prefers_complete_env_credentials_over_the_auth_file() {
        let env = creds("env-id", "env-secret");
        let file = creds("file-id", "file-secret");
        let auth = detect_with(Some(env), Some(file)).expect("complete env pair wins");
        assert_eq!(auth.client_id, "env-id");
    }

    #[test]
    fn detect_falls_back_to_the_auth_file_when_env_pair_is_incomplete() {
        let half_env = AppCredentials {
            client_id: "env-id".to_string(),
            ..creds("", "")
        };
        let auth =
            detect_with(Some(half_env), Some(creds("file-id", "file-secret"))).expect("file pair");
        assert_eq!(auth.client_id, "file-id");
    }

    #[test]
    fn missing_credentials_produce_an_error_state_not_a_backend() {
        // There is deliberately no fallback identity: with nothing (or only
        // half of a pair) configured, detect must yield NO auth at all —
        // never a working-looking backend.
        assert!(detect_with(None, None).is_none());
        assert!(detect_with(Some(creds("only-an-id", "")), None).is_none());
        assert!(detect_with(None, Some(creds("", "only-a-secret"))).is_none());
    }

    // The path-injected save/load roundtrips moved with the storage module
    // into hyprlay-core::credentials; only the resolution precedence above
    // is daemon-owned.
}
