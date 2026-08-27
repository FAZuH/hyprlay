//! Sibling-binary resolution and exec. The four hyprlay binaries are
//! installed side by side; the launcher finds them next to its own image
//! (via `current_exe().parent()`), never on `$PATH`, so a partially
//! upgraded install fails loudly instead of mixing versions.

use std::path::Path;
use std::path::PathBuf;

pub const DAEMON_BIN: &str = "hyprlayd";
pub const GUI_BIN: &str = "hyprlay-gui";
pub const TRAY_BIN: &str = "hyprlay-tray";

/// Resolve `name` inside `exe_dir`, demanding that the file exists so a
/// broken install reports here instead of exec-failing with a bare ENOENT.
pub fn resolve_sibling(exe_dir: &Path, name: &str) -> Result<PathBuf, String> {
    let path = exe_dir.join(name);
    if path.exists() {
        Ok(path)
    } else {
        Err(format!(
            "error: {name} not found next to hyprlay (expected {})\nthe hyprlay binaries must be installed together",
            path.display()
        ))
    }
}

/// Replace this process with the sibling binary of `name`. On success this
/// never returns (the image becomes the sibling's); on failure it prints
/// the resolution/exec error and returns exit code 1.
pub fn exec_sibling(name: &str) -> i32 {
    use std::os::unix::process::CommandExt;

    // exec() keeps our PID, so supervisors that track the launcher by PID
    // or cgroup keep tracking the daemon/GUI it becomes.
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            eprintln!("error: could not locate the running hyprlay binary: {e}");
            return 1;
        }
    };
    let Some(dir) = exe.parent() else {
        eprintln!("error: could not find the directory of {}", exe.display());
        return 1;
    };
    match resolve_sibling(dir, name) {
        Ok(path) => {
            let err = std::process::Command::new(path).exec();
            eprintln!("error: could not start {name}: {err}");
            1
        }
        Err(message) => {
            eprintln!("{message}");
            1
        }
    }
}
