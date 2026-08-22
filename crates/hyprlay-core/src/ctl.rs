//! Client side of the control socket and the wire-protocol surface shared
//! by every binary: the socket path, the startup probe, the blocking
//! one-line client, and the `help` reply text. The daemon-side listener
//! lives in the daemon binary; it speaks this same vocabulary.

use std::path::PathBuf;

/// Outcome of probing the control-socket path before the daemon binds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketProbe {
    /// A live listener accepted the connection: another daemon owns it.
    AlreadyRunning,
    /// Nobody answered but a socket file remained; it was removed as stale.
    StaleRemoved,
    /// Nothing was listening and nothing needed removing.
    Free,
}

/// Probe the control socket by connecting like any client would. Plain fn
/// (path in, verdict out) so startup can decide before anything is bound
/// and unit tests can point it at temp-dir sockets.
///
/// No timeout around the connect: an AF_UNIX connect never blocks on a
/// live listener — it succeeds immediately or fails with ECONNREFUSED
/// (stale file behind the path) or ENOENT (absent), which is exactly the
/// information this decision needs.
pub fn probe_socket(path: &std::path::Path) -> SocketProbe {
    use std::io::ErrorKind;
    match std::os::unix::net::UnixStream::connect(path) {
        // Only a live listener accepts; that is the double-start signal.
        Ok(_) => SocketProbe::AlreadyRunning,
        Err(e) if e.kind() == ErrorKind::ConnectionRefused => match std::fs::remove_file(path) {
            Ok(()) => SocketProbe::StaleRemoved,
            // Someone else cleaned it between our connect and unlink.
            Err(e) if e.kind() == ErrorKind::NotFound => SocketProbe::Free,
            // Nothing we can fix here; the bind below surfaces the real
            // problem through its own warn path if the path stays occupied.
            Err(e) => {
                tracing::warn!(
                    event = "ctl_stale_unlink_failed",
                    error = %e,
                    path = %path.display(),
                    "could not remove stale control socket"
                );
                SocketProbe::Free
            }
        },
        // Absent, or an error we cannot attribute to a live daemon: let
        // the bind decide.
        Err(_) => SocketProbe::Free,
    }
}

/// Control socket: the daemon listens on a unix stream socket so keybinds
/// can drive the overlay through the same binary:
///
///   hyprlay set position          # cycle corner presets
///   hyprlay set talking-only on   # any config key
///   hyprlay move left             # re-anchor to an edge
///   hyprlay status
///
/// Protocol: one command line in, one reply line out, connection closed.
pub fn socket_path() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("hyprlay.sock")
}

/// Send one command line to the daemon and read its reply. Blocking, on a
/// plain std UnixStream — callers that must not stall a UI thread wrap
/// this in their own off-thread mechanism.
pub fn send_command_line(command: &str) -> Option<String> {
    use std::io::Read;
    use std::io::Write;
    let path = socket_path();
    let mut stream = std::os::unix::net::UnixStream::connect(&path)
        .map_err(|e| {
            eprintln!("error: cannot connect to daemon at {}: {e}", path.display());
            eprintln!("is the overlay running?");
        })
        .ok()?;
    stream
        .write_all(format!("{command}\n").as_bytes())
        .map_err(|_| eprintln!("error: failed to send command"))
        .ok()?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut reply = String::new();
    stream
        .read_to_string(&mut reply)
        .map_err(|_| eprintln!("error: failed to read reply"))
        .ok()?;
    Some(reply)
}

/// EXAMPLES section of the help text, shared verbatim by the wire `help`
/// reply and the CLI's root help so the two surfaces cannot drift.
pub fn examples_section() -> String {
    "EXAMPLES (Hyprland):
    bind = SUPER, F9,  exec, hyprlay set talking-only
    bind = SUPER, F10, exec, hyprlay set position
    bind = SUPER, F11, exec, hyprlay set opacity 80
"
    .to_string()
}

/// FILES section of the help text, shared verbatim by the wire `help`
/// reply and the CLI's root help so the two surfaces cannot drift.
pub fn files_section() -> String {
    "FILES:
    ~/.config/hyprlay/config.toml   config (TOML sections, clamped on load)
    ~/.config/hyprlay/token.json    cached OAuth token (0600)
    $XDG_STATE_HOME/hyprlay/logs/   JSON event logs (daily rotation)
"
    .to_string()
}

/// Local usage text — printed without contacting the daemon. Also the
/// exact body of the wire `help` reply, so it is pinned by the same tests.
pub fn usage() -> String {
    // Range hints come from the shared bounds table so help can never drift
    // from what the parser actually accepts.
    let keys = keys_table();
    format!(
        "hyprlay {} — lightweight Discord voice overlay (Wayland/Hyprland)

USAGE:
    hyprlay                      run the overlay daemon
    hyprlay daemon               run the overlay daemon (explicit)
    hyprlay gui                  open the settings window
    hyprlay get <key>            read one setting
    hyprlay set <key> [value]    change one setting; omit value to
                                 cycle enums / flip flags
    hyprlay <COMMAND>            other commands below
    hyprlay -h | --help          this help
    hyprlay -V | --version       print version

COMMANDS:
    status                       connection + config summary
    dump                         live runtime config as TOML
    move <left|right|center|top|bottom>
                                 re-anchor to a screen edge
    nudge <dx> <dy>              shift by pixels (negative allowed)
    reset [position|layout|opacity|colors]
                                 reset all or one group to defaults
                                 (always keeps the monitor choice)
    save                         persist runtime config to config.toml
    reload                       re-read config.toml
    restart                      re-exec the daemon (applies new credentials)
    quit                         stop the running daemon cleanly
    monitors                     list outputs

KEYS (use with get/set):
{keys}
{examples}
{files}",
        env!("CARGO_PKG_VERSION"),
        examples = examples_section(),
        files = files_section()
    )
}

/// The KEYS table rows, shared by the wire `help` reply and the CLI's
/// `set`/`get` help: one formatter, so the two surfaces cannot drift.
pub fn keys_table() -> String {
    crate::domain::Key::ALL
        .iter()
        .map(|k| format!("      {:<14} {}\n", k.name(), group_hint(k.group())))
        .collect()
}

fn group_hint(group: crate::domain::Group) -> &'static str {
    match group {
        crate::domain::Group::Position => "corner preset, glue edge, offsets, rtl, output",
        crate::domain::Group::Layout => "width, scale, sizes, spacing, filters, visibility",
        crate::domain::Group::Opacity => "overall and per-part opacity percents",
        crate::domain::Group::Colors => "speaking ring, username text and chip colors",
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::Key;

    /// The KEYS table is shared between the wire `help` reply and the CLI's
    /// `set`/`get` help, so its row shape is pinned here once.
    #[test]
    fn keys_table_lists_every_key_in_order_with_group_hints() {
        let table = super::keys_table();
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), Key::ALL.len(), "one row per key");
        for (line, key) in lines.iter().zip(Key::ALL) {
            let expected = format!(
                "      {:<14} {}",
                key.name(),
                super::group_hint(key.group())
            );
            assert_eq!(*line, expected, "row for {}", key.name());
        }
        // No duplication drift: every key owns exactly one row prefix.
        for name in Key::ALL.iter().map(|k| k.name()) {
            let row = format!("      {:<14} ", name);
            assert_eq!(
                table.matches(&row).count(),
                1,
                "{name} must appear exactly once"
            );
        }
    }

    /// `usage()` is the body of the wire `help` reply: byte-stable contract.
    /// This pins the section skeleton so the keys-table extraction cannot
    /// silently reshape it.
    #[test]
    fn usage_keeps_the_wire_help_section_shape() {
        let text = super::usage();
        let mut lines = text.lines();
        assert_eq!(
            lines.next(),
            Some(concat!(
                "hyprlay ",
                env!("CARGO_PKG_VERSION"),
                " — lightweight Discord voice overlay (Wayland/Hyprland)"
            )),
            "banner line is the wire-visible first line"
        );
        let headings = [
            "USAGE:",
            "COMMANDS:",
            "KEYS (use with get/set):",
            "EXAMPLES (Hyprland):",
            "FILES:",
        ];
        let mut cursor = 0;
        for heading in headings {
            let pos = text[cursor..].find(heading).unwrap_or_else(|| {
                panic!("heading {heading} missing or out of order");
            }) + cursor;
            cursor = pos + heading.len();
        }
        assert!(text.contains("quit"), "the quit command must be listed");
        for key in Key::ALL {
            assert!(text.contains(key.name()), "key {} listed", key.name());
        }
    }
}
