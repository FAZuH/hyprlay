//! Avatar fetching: blocking HTTP on the tokio blocking pool, cached by the
//! caller. Discord CDN serves PNG at fixed sizes; 64px is plenty for a 40px
//! avatar even at 200% scale.

use std::time::Duration;

pub fn url_for(user_id: &str, hash: &str) -> String {
    format!("https://cdn.discordapp.com/avatars/{user_id}/{hash}.png?size=64")
}

pub async fn fetch(user_id: String, hash: String, url: String) -> Option<Vec<u8>> {
    tokio::task::spawn_blocking(move || {
        if let Some(bytes) = super::cache::load_avatar(&user_id, &hash) {
            return Some(bytes);
        }
        let resp = match ureq::get(&url).timeout(Duration::from_secs(10)).call() {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(event = "avatar_fetch_failed", user_id = %user_id, error = %e, "cdn request failed");
                return None;
            }
        };
        let mut bytes = Vec::new();
        use std::io::Read;
        resp.into_reader().take(1 << 20).read_to_end(&mut bytes).ok()?;
        super::cache::store_avatar(&user_id, &hash, &bytes);
        Some(bytes)
    })
    .await
    .ok()
    .flatten()
}
