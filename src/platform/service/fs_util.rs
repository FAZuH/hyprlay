//! File-system helpers shared by the service backends: the identical
//! create-parent + write and reported-remove sequences the systemd,
//! launchd, and Windows adapters each used to repeat.

use std::fs;
use std::path::Path;

use hyprlay_core::daemon_control::ServiceError;

pub(super) fn write_file(path: &Path, contents: &[u8]) -> Result<(), ServiceError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ServiceError::CreateDirFailed {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(path, contents).map_err(|source| ServiceError::WriteFileFailed {
        path: path.to_path_buf(),
        source,
    })
}

pub(super) fn remove_reported(path: &Path, report: &mut Vec<String>) -> Result<(), ServiceError> {
    match fs::remove_file(path) {
        Ok(()) => report.push(format!("removed {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            report.push(format!("already absent {}", path.display()))
        }
        Err(source) => {
            return Err(ServiceError::RemoveFileFailed {
                path: path.to_path_buf(),
                source,
            });
        }
    }
    Ok(())
}
