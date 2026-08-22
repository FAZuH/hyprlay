//! A fresh directory per call: cargo runs tests in parallel threads, so a
//! shared path would be shared mutable state. Cleaned up by the caller.

use std::path::PathBuf;

pub fn unique_temp_dir(tag: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "hyprlay-test-{}-{}-{tag}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
