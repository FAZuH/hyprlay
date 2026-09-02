//! Sibling-binary resolution and exec. Only the daemon is a separate
//! binary now; the launcher (`hyprlay`) finds it next to its own image
//! (via `current_exe().parent()`), never on `$PATH`, so a partially
//! upgraded install fails loudly instead of mixing versions.

use std::path::Path;
use std::path::PathBuf;

pub use hyprlay_core::bins::DAEMON_BIN;

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
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                let err = std::process::Command::new(path).exec();
                eprintln!("error: could not start {name}: {err}");
                1
            }
            #[cfg(not(unix))]
            {
                // Windows has no `exec`. Approximate re-exec by spawning the
                // sibling detached and exiting, so the launcher is a short
                // stepping stone. The child inherits the launcher's stdio
                // handles; the PID changes (no exec), which a PID- or
                // cgroup-tracking supervisor would notice.
                match std::process::Command::new(path).spawn() {
                    Ok(_child) => std::process::exit(0),
                    Err(e) => {
                        eprintln!("error: could not start {name}: {e}");
                        1
                    }
                }
            }
        }
        Err(message) => {
            eprintln!("{message}");
            1
        }
    }
}
