//! The on-disk binary's version, for the tray's restart-for-the-new-build
//! notice.
//!
//! Asking the running daemon and the installed binary the same question is
//! the only honest drift check: a service-managed daemon keeps running the
//! old build after an update until it restarts. The answer is cached against
//! the exe's mtime, so the 2 s poll loop spawns `--version` once per build
//! instead of every tick.

use std::path::Path;
use std::process::Command;
use std::sync::Mutex;
use std::time::SystemTime;

/// Last read, keyed by the exe's modification time.
static CACHE: Mutex<Option<(SystemTime, String)>> = Mutex::new(None);

/// The installed binary's version token, e.g. `0.4.0`, or `None` when the
/// exe cannot be located or run.
pub fn on_disk_version() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let mtime = std::fs::metadata(&exe).ok()?.modified().ok()?;
    if let Ok(cache) = CACHE.lock()
        && let Some((cached_mtime, version)) = cache.as_ref()
        && *cached_mtime == mtime
    {
        return Some(version.clone());
    }
    let version = run_version(&exe)?;
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((mtime, version.clone()));
    }
    Some(version)
}

/// Run `<exe> --version` and take its last whitespace-separated token, which
/// is the version in clap's `name version` output. Any spawn failure is
/// `None`: the tray shows no notice rather than a wrong one.
fn run_version(exe: &Path) -> Option<String> {
    let mut cmd = Command::new(exe);
    cmd.arg("--version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashes in a tray process.
        cmd.creation_flags(0x08000000);
    }
    let output = cmd.output().ok()?;
    let stdout = String::from_utf8(output.stdout).ok()?;
    stdout.split_whitespace().last().map(str::to_string)
}
