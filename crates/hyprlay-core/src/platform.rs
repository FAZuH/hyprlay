//! Platform context facade: the small set of OS facts the pure domain needs
//! (runtime dir, state dir, process spawn, secure file permissions), plus the
//! [`Platform`] port that the host package resolves once and injects into the
//! fronts.
//!
//! The pure crate stays platform-neutral: path resolution uses `dirs`, and the
//! only POSIX-mode logic is the cfg-gated [`secure_perms`] helper (a no-op on
//! platforms without a mode concept). Detached process spawn — inherently
//! OS-specific — lives behind the [`Platform`] trait, implemented by the host
//! package's `src/platform/host.rs`.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

/// Runtime directory for socket / pipe / lock files
/// (`$XDG_RUNTIME_DIR`, falling back to `$TMPDIR`, which is also the answer on
/// macOS where `dirs::runtime_dir()` is `None`).
pub fn runtime_dir() -> PathBuf {
    dirs::runtime_dir().unwrap_or_else(std::env::temp_dir)
}

/// State directory for durable runtime state (logs). `None` when neither the
/// platform state dir nor the local data dir is available.
pub fn state_dir() -> Option<PathBuf> {
    dirs::state_dir().or_else(dirs::data_local_dir)
}

/// Restrict a file to the owning user (0600 on unix). On platforms without a
/// POSIX mode concept this is a documented no-op that relies on the user's
/// profile ACLs (see the cross-platform ADR): the call still exists so a
/// caller cannot forget that a secret file needs restricting.
pub fn secure_perms(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// The platform context resolved once per process and injected into the
/// fronts. [`spawn`](Self::spawn) has no portable default — detaching a child
/// is inherently OS-specific; the remaining methods default to the pure
/// helpers above and may be overridden by the host.
pub trait Platform: Send + Sync {
    /// Spawn `cmd` detached from this process: its own process group, null
    /// stdio, no wait, so the child outlives its parent.
    fn spawn(&self, cmd: &mut Command) -> std::io::Result<()>;

    fn runtime_dir(&self) -> PathBuf {
        runtime_dir()
    }

    fn state_dir(&self) -> Option<PathBuf> {
        state_dir()
    }

    fn secure_perms(&self, path: &Path) -> std::io::Result<()> {
        secure_perms(path)
    }
}
