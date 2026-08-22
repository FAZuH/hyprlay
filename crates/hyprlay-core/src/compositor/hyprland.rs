//! Hyprland adapter: monitor discovery via `hyprctl monitors -j`.

use super::Compositor;
use super::Monitor;

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
            })
            .collect()
    }
}

/// Marker type for the Hyprland adapter; all behavior lives in the trait impl.
pub struct Hyprland;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_compositor_reports_no_monitors() {
        assert!(super::super::Unknown.monitors().is_empty());
    }
}
