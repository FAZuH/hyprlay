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
                x: m["x"].as_i64().unwrap_or(0) as i32,
                y: m["y"].as_i64().unwrap_or(0) as i32,
                width: m["width"].as_i64().unwrap_or(0) as i32,
                height: m["height"].as_i64().unwrap_or(0) as i32,
            })
            .collect()
    }
}

pub fn cursor_pos() -> Option<(i32, i32)> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    let sig = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")?;
    let socket_path = std::path::PathBuf::from(runtime)
        .join("hypr")
        .join(sig)
        .join(".socket.sock");
    let mut stream = match std::os::unix::net::UnixStream::connect(&socket_path) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(event = "cursor_socket_failed", error = %e, "could not connect to hyprland socket");
            return None;
        }
    };
    if let Err(e) = std::io::Write::write_all(&mut stream, b"cursorpos\n") {
        tracing::debug!(event = "cursor_send_failed", error = %e, "could not send cursorpos");
        return None;
    }
    let mut buf = Vec::new();
    if let Err(e) = std::io::Read::read_to_end(&mut stream, &mut buf) {
        tracing::debug!(event = "cursor_read_failed", error = %e, "could not read cursorpos reply");
        return None;
    }
    let reply = String::from_utf8_lossy(&buf).trim().to_string();
    if reply.is_empty() {
        tracing::debug!(event = "cursor_empty_reply", "empty cursorpos reply");
        return None;
    }
    if reply.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&reply) {
            let x = v.get("x").and_then(|x| x.as_i64()).map(|x| x as i32);
            let y = v.get("y").and_then(|y| y.as_i64()).map(|y| y as i32);
            if let (Some(x), Some(y)) = (x, y) {
                return Some((x, y));
            }
        }
        tracing::debug!(event = "cursor_parse_failed", reply = %reply, "failed to parse json cursorpos");
        return None;
    }
    let parts: Vec<&str> = reply.split(',').collect();
    if parts.len() == 2
        && let (Ok(x), Ok(y)) = (
            parts[0].trim().parse::<i32>(),
            parts[1].trim().parse::<i32>(),
        )
    {
        return Some((x, y));
    }
    tracing::debug!(event = "cursor_parse_failed", reply = %reply, "failed to parse cursorpos");
    None
}

#[allow(dead_code)]
pub(crate) fn parse_cursor_reply(reply: &str) -> Option<(i32, i32)> {
    let reply = reply.trim();
    if reply.is_empty() {
        return None;
    }
    if reply.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(reply) {
            let x = v.get("x").and_then(|x| x.as_i64()).map(|x| x as i32);
            let y = v.get("y").and_then(|y| y.as_i64()).map(|y| y as i32);
            if let (Some(x), Some(y)) = (x, y) {
                return Some((x, y));
            }
        }
        return None;
    }
    let parts: Vec<&str> = reply.split(',').collect();
    if parts.len() == 2
        && let (Ok(x), Ok(y)) = (
            parts[0].trim().parse::<i32>(),
            parts[1].trim().parse::<i32>(),
        )
    {
        return Some((x, y));
    }
    None
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

    #[test]
    fn cursor_reply_parses_comma_form() {
        assert_eq!(parse_cursor_reply("100, 200"), Some((100, 200)));
        assert_eq!(parse_cursor_reply("  -10 ,  20  "), Some((-10, 20)));
        assert_eq!(parse_cursor_reply("0,0"), Some((0, 0)));
    }

    #[test]
    fn cursor_reply_parses_json_form() {
        assert_eq!(
            parse_cursor_reply(r#"{"x": 123, "y": 456}"#),
            Some((123, 456))
        );
        assert_eq!(parse_cursor_reply(r#"{"x":-5,"y":10}"#), Some((-5, 10)));
    }

    #[test]
    fn cursor_reply_rejects_garbage() {
        assert_eq!(parse_cursor_reply(""), None);
        assert_eq!(parse_cursor_reply("not a point"), None);
        assert_eq!(parse_cursor_reply(r#"{"a":1}"#), None);
    }

    #[test]
    fn cursor_pos_returns_none_without_hyprland_env() {
        let orig_runtime = std::env::var_os("XDG_RUNTIME_DIR");
        let orig_sig = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE");
        unsafe {
            std::env::remove_var("XDG_RUNTIME_DIR");
            std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");
        }
        assert_eq!(cursor_pos(), None);
        assert_eq!(super::super::cursor_pos(), None);
        if let Some(v) = orig_runtime {
            unsafe { std::env::set_var("XDG_RUNTIME_DIR", v) };
        }
        if let Some(v) = orig_sig {
            unsafe { std::env::set_var("HYPRLAND_INSTANCE_SIGNATURE", v) };
        }
    }
}
