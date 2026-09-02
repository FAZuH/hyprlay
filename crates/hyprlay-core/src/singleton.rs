//! Single-instance guard via an advisory cross-platform file lock
//! (`fd_lock::RwLock` — `flock` on unix, `LockFileEx` on Windows).
//!
//! The GUI and tray fronts each take one at startup so a second copy fails
//! fast instead of silently competing for the same DBus name / overlay. The
//! lock self-releases when the owning process exits (the file is closed), so
//! a crash can never leave a stale lock behind.

use std::fs::File;
use std::fs::OpenOptions;
use std::path::Path;
use std::path::PathBuf;

use fd_lock::RwLock;
use fd_lock::RwLockWriteGuard;

/// A held single-instance lock. The lock guard is kept for the process
/// lifetime; dropping it releases the lock, which is exactly how the tests
/// drive contention.
pub struct Singleton {
    // The lock guard is kept alive for the process lifetime. The underlying
    // `RwLock` is leaked to give the guard a `'static` borrow without a
    // self-reference; that is one extra fd held as long as the singleton
    // itself, closed at process exit.
    _guard: RwLockWriteGuard<'static, File>,
    #[allow(dead_code)]
    path: PathBuf,
}

/// Why a singleton could not be acquired.
#[derive(Debug)]
pub enum AcquireError {
    /// Another process already holds the lock: a live instance is running.
    AlreadyHeld,
    /// The lock file could not be opened or the lock failed for a reason
    /// other than contention.
    Io(std::io::Error),
}

impl PartialEq for AcquireError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::AlreadyHeld, Self::AlreadyHeld) => true,
            (Self::Io(a), Self::Io(b)) => a.kind() == b.kind(),
            _ => false,
        }
    }
}

impl std::fmt::Display for AcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyHeld => write!(f, "another instance is already running"),
            Self::Io(e) => write!(f, "could not acquire lock: {e}"),
        }
    }
}

impl std::error::Error for AcquireError {}

impl Singleton {
    /// The lock file path for `name` under `runtime_dir`
    /// (`<runtime_dir>/<name>.lock`). Pure so tests can point it at a
    /// tempdir rather than the real runtime dir.
    pub fn path_for(runtime_dir: &Path, name: &str) -> PathBuf {
        runtime_dir.join(format!("{name}.lock"))
    }
}

/// Acquire the singleton lock at `<runtime_dir>/<name>.lock`.
///
/// Opens (creating if needed) the lock file and takes a non-blocking
/// exclusive lock. Returns [`AcquireError::AlreadyHeld`] if another process
/// owns it; otherwise the held [`Singleton`] keeps it until dropped.
///
/// The lock is keyed on the open file description, so two independent opens
/// of the same path — even inside one process — contend correctly. That is
/// exactly what the unit tests rely on.
pub fn acquire_at(runtime_dir: &Path, name: &str) -> Result<Singleton, AcquireError> {
    let path = Singleton::path_for(runtime_dir, name);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(AcquireError::Io)?;
    // Leak the lock so its guard can borrow it for `'static`: the singleton
    // lives as long as the process, so the extra fd is intended (it is what
    // keeps the lock held until exit).
    let lock: &'static mut RwLock<File> = Box::leak(Box::new(RwLock::new(file)));
    match lock.try_write() {
        Ok(guard) => Ok(Singleton {
            _guard: guard,
            path,
        }),
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Err(AcquireError::AlreadyHeld),
        Err(e) => Err(AcquireError::Io(e)),
    }
}

/// Acquire the singleton lock under the platform runtime dir
/// (`$XDG_RUNTIME_DIR`, falling back to `$TMPDIR`), the production path.
pub fn acquire(name: &str) -> Result<Singleton, AcquireError> {
    let dir = crate::platform::runtime_dir();
    acquire_at(&dir, name)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// A fresh, call-unique tempdir so no two tests share a lock-file path
    /// (lock contention is the behavior under test; a shared path would let
    /// a parallel test's held lock wrongly fail another test's reacquire).
    fn scratch() -> PathBuf {
        use std::sync::atomic::AtomicU64;
        use std::sync::atomic::Ordering;
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("hyprlay-lock-{}-{}", std::process::id(), n));
        fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn acquire_succeeds_when_no_other_holder() {
        let dir = scratch();
        assert!(
            acquire_at(&dir, "gui").is_ok(),
            "a free lock must be acquired"
        );
    }

    #[test]
    fn second_acquire_on_the_same_name_fails_with_already_held() {
        let dir = scratch();
        let first = acquire_at(&dir, "gui").expect("first acquire");
        assert!(
            matches!(acquire_at(&dir, "gui"), Err(AcquireError::AlreadyHeld)),
            "second acquire on a held name must be AlreadyHeld"
        );
        // Dropping the first must release the underlying lock.
        drop(first);
    }

    #[test]
    fn dropping_the_holder_frees_the_lock_for_reacquire() {
        let dir = scratch();
        let first = acquire_at(&dir, "gui").expect("first acquire");
        assert!(
            matches!(acquire_at(&dir, "gui"), Err(AcquireError::AlreadyHeld)),
            "held lock must reject a second acquirer"
        );
        drop(first);
        // The released lock can be taken again.
        assert!(
            acquire_at(&dir, "gui").is_ok(),
            "lock must be free after the holder drops"
        );
    }

    #[test]
    fn distinct_names_never_conflict() {
        let dir = scratch();
        let gui = acquire_at(&dir, "gui");
        let tray = acquire_at(&dir, "tray");
        assert!(gui.is_ok(), "gui lock acquired");
        assert!(tray.is_ok(), "tray lock is independent of gui");
    }

    #[test]
    fn path_for_builds_a_dot_lock_name() {
        let path = Singleton::path_for(std::path::Path::new("/run/user/1000"), "hyprlay-gui");
        assert_eq!(
            path,
            std::path::Path::new("/run/user/1000/hyprlay-gui.lock")
        );
    }

    #[test]
    fn io_error_is_surfaced_not_confused_with_contention() {
        // A path whose parent does not exist forces an open failure that is
        // about the filesystem, not another holder.
        let dir = scratch().join("no").join("such").join("dir");
        let result = acquire_at(&dir, "gui");
        assert!(
            matches!(result, Err(AcquireError::Io(_))),
            "missing parent must be an Io error"
        );
    }
}
