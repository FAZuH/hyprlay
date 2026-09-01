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
                scale: m["scale"].as_f64().unwrap_or(1.0) as f32,
            })
            .collect()
    }
}

pub fn cursor_pos() -> Option<(i32, i32)> {
    for path in candidate_socket_paths() {
        if let Some(pos) = cursor_pos_from_socket(&path) {
            return Some(pos);
        }
    }
    cursor_pos_via_hyprctl()
}

pub fn has_socket() -> bool {
    candidate_socket_paths().iter().any(|p| p.exists())
}

fn candidate_socket_paths() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;
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

fn cursor_pos_from_socket(path: &std::path::Path) -> Option<(i32, i32)> {
    use std::io::Read;
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;
    let mut stream = match UnixStream::connect(path) {
        Ok(s) => s,
        Err(e) => {
            tracing::debug!(event = "cursor_socket_failed", path = %path.display(), error = %e, "could not connect to hyprland socket");
            return None;
        }
    };
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
    if let Err(e) = stream.write_all(b"cursorpos\n") {
        tracing::debug!(event = "cursor_send_failed", error = %e, "could not send cursorpos");
        return None;
    }
    let mut buf = Vec::new();
    if let Err(e) = stream.read_to_end(&mut buf) {
        tracing::debug!(event = "cursor_read_failed", error = %e, "could not read cursorpos reply");
        return None;
    }
    let reply = String::from_utf8_lossy(&buf).trim().to_string();
    if reply.is_empty() {
        tracing::debug!(event = "cursor_empty_reply", path = %path.display(), "empty cursorpos reply");
        return None;
    }
    if let Some(pos) = parse_cursor_reply(&reply) {
        return Some(pos);
    }
    tracing::debug!(event = "cursor_parse_failed", reply = %reply, "failed to parse cursorpos");
    None
}

fn cursor_pos_via_hyprctl() -> Option<(i32, i32)> {
    for args in [vec!["cursorpos", "-j"], vec!["cursorpos"]] {
        if let Ok(out) = std::process::Command::new("hyprctl").args(&args).output()
            && out.status.success()
        {
            let reply = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !reply.is_empty() {
                if let Some(pos) = parse_cursor_reply(&reply) {
                    return Some(pos);
                }
                tracing::debug!(event = "cursor_hyprctl_parse_failed", reply = %reply, "hyprctl parse failed");
            }
        }
    }
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
        let tmp = std::env::temp_dir().join(format!("hyprlay-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        unsafe {
            std::env::set_var("XDG_RUNTIME_DIR", &tmp);
            std::env::remove_var("HYPRLAND_INSTANCE_SIGNATURE");
        }
        assert_eq!(cursor_pos(), None);
        assert_eq!(super::super::cursor_pos(), None);
        if let Some(v) = orig_runtime {
            unsafe { std::env::set_var("XDG_RUNTIME_DIR", v) };
        } else {
            unsafe { std::env::remove_var("XDG_RUNTIME_DIR") };
        }
        if let Some(v) = orig_sig {
            unsafe { std::env::set_var("HYPRLAND_INSTANCE_SIGNATURE", v) };
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn monitor_scale_parsed_and_defaults_to_one() {
        let json = serde_json::json!([{
            "name": "eDP-1",
            "description": "test",
            "focused": true,
            "x": 0,
            "y": 0,
            "width": 1920,
            "height": 1200,
            "scale": 1.25
        }, {
            "name": "HDMI-A-1",
            "description": "test2",
            "focused": false,
            "x": 0,
            "y": 0,
            "width": 1920,
            "height": 1080
        }]);
        let arr = json.as_array().unwrap();
        let m0 = &arr[0];
        let m1 = &arr[1];
        let scale0 = m0["scale"].as_f64().unwrap_or(1.0) as f32;
        let scale1 = m1["scale"].as_f64().unwrap_or(1.0) as f32;
        assert!((scale0 - 1.25).abs() < f32::EPSILON);
        assert!((scale1 - 1.0).abs() < f32::EPSILON);
        let mon0 = Monitor {
            name: m0["name"].as_str().unwrap().to_string(),
            description: String::new(),
            active: true,
            x: m0["x"].as_i64().unwrap() as i32,
            y: 0,
            width: m0["width"].as_i64().unwrap() as i32,
            height: m0["height"].as_i64().unwrap() as i32,
            scale: scale0,
        };
        assert!((mon0.scale - 1.25).abs() < f32::EPSILON);
        assert_eq!((mon0.width as f32 / mon0.scale) as i32, 1536);
    }
}
