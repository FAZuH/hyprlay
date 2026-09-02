//! Integration tests for the ctl startup probe: every case binds real
//! unix listeners and removes real temp-dir entries, so it belongs here
//! instead of the unit-test module. The probe is transport-agnostic, so
//! these drive it over a test-only unix [`ControlEndpoint`].

mod common;

use std::io::Read;
use std::io::Write;
use std::os::unix::net::UnixListener;
use std::os::unix::net::UnixStream;
use std::path::Path;

use common::unique_temp_dir;
use hyprlay_core::ctl::ControlEndpoint;
use hyprlay_core::ctl::ControlStream;
use hyprlay_core::ctl::SocketProbe;
use hyprlay_core::ctl::probe_socket;

/// Test-only control endpoint backed by a unix stream socket.
struct UnixControl;

impl ControlEndpoint for UnixControl {
    fn connect(&self, path: &Path) -> std::io::Result<Box<dyn ControlStream>> {
        Ok(Box::new(Stream(UnixStream::connect(path)?)))
    }
}

struct Stream(UnixStream);

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl ControlStream for Stream {
    fn shutdown_write(&mut self) -> std::io::Result<()> {
        self.0.shutdown(std::net::Shutdown::Write)
    }
}

#[test]
fn live_listener_makes_the_guard_report_running() {
    let dir = unique_temp_dir("live");
    let path = dir.join("hyprlay.sock");
    let listener = UnixListener::bind(&path).unwrap();

    assert_eq!(
        probe_socket(&UnixControl, &path),
        SocketProbe::AlreadyRunning
    );
    // A live daemon's socket must survive the probe untouched.
    assert!(path.exists());
    drop(listener);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn stale_socket_file_is_removed_and_bind_proceeds() {
    let dir = unique_temp_dir("stale");
    let path = dir.join("hyprlay.sock");
    // Bind then drop the listener: exactly the state a crashed daemon
    // leaves behind (file on disk, nobody listening).
    drop(UnixListener::bind(&path).unwrap());
    assert!(path.exists());

    assert_eq!(probe_socket(&UnixControl, &path), SocketProbe::StaleRemoved);
    assert!(!path.exists());
    // And the freed path accepts a fresh bind.
    let rebound = UnixListener::bind(&path);
    assert!(rebound.is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn absent_socket_reports_free() {
    let dir = unique_temp_dir("absent");
    assert_eq!(
        probe_socket(&UnixControl, &dir.join("hyprlay.sock")),
        SocketProbe::Free
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
