//! Hyprland compositor adapter: monitor discovery via `hyprctl monitors -j`,
//! plus the Hyprland IPC socket-path probe used to detect a live session.
//! The cursor query lives in the sibling `platform::cursor::hyprland`.

use std::path::PathBuf;

use hyprlay_core::compositor::Compositor;
use hyprlay_core::compositor::Monitor;

/// Marker type for the Hyprland adapter; all behavior lives in the trait impl.
pub struct Hyprland;

impl Compositor for Hyprland {
    fn monitors(&self) -> Vec<Monitor> {
        let Some(v) = monitors_json() else {
            return Vec::new();
        };
        let Some(arr) = v.as_array() else {
            tracing::debug!(
                event = "hyprctl_parse_failed",
                "unexpected hyprctl output shape"
            );
            return Vec::new();
        };
        arr.iter()
            .map(|m| Monitor {
                name: m["name"].as_str().unwrap_or("?").to_string(),
                description: m["description"].as_str().unwrap_or("").to_string(),
                active: m["current"].as_bool().unwrap_or(false)
                    || m["focused"] == serde_json::json!(true),
                x: m["x"].as_i64().unwrap_or(0) as i32,
                y: m["y"].as_i64().unwrap_or(0) as i32,
                width: m["width"].as_i64().unwrap_or(0) as i32,
                height: m["height"].as_i64().unwrap_or(0) as i32,
                scale: m["scale"].as_f64().unwrap_or(1.0) as f32,
            })
            .collect()
    }
}

/// Whether a live Hyprland IPC socket is present under this session.
pub fn has_socket() -> bool {
    candidate_socket_paths().iter().any(|p| p.exists())
}

/// Candidate Hyprland IPC socket paths in probe order. Reused by the cursor
/// adapter to read the pointer position.
pub(crate) fn candidate_socket_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let (Some(rt), Some(sig)) = (
        std::env::var_os("XDG_RUNTIME_DIR"),
        std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE"),
    ) {
        paths.push(
            PathBuf::from(rt)
                .join("hypr")
                .join(sig)
                .join(".socket.sock"),
        );
    }
    let mut bases = Vec::new();
    if let Some(rt) =
        dirs::runtime_dir().or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from))
    {
        bases.push(rt.join("hypr"));
    } else {
        let uid = unsafe { libc::getuid() };
        bases.push(PathBuf::from(format!("/run/user/{uid}/hypr")));
        bases.push(PathBuf::from("/tmp/hypr"));
    }
    for base in bases {
        if let Ok(entries) = std::fs::read_dir(&base) {
            for entry in entries.flatten() {
                let p = entry.path().join(".socket.sock");
                if p.exists() && !paths.contains(&p) {
                    paths.push(p);
                }
            }
        }
    }
    paths
}

fn monitors_json() -> Option<serde_json::Value> {
    let out = match std::process::Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
    {
        Ok(out) => out,
        Err(e) => {
            tracing::debug!(event = "hyprctl_failed", error = %e, "could not run hyprctl");
            return None;
        }
    };
    match serde_json::from_slice(&out.stdout) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::debug!(event = "hyprctl_parse_failed", error = %e, "unexpected hyprctl output");
            None
        }
    }
}
