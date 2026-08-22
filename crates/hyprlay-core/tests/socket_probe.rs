//! Integration tests for the ctl startup probe: every case binds real
//! unix listeners and removes real temp-dir entries, so it belongs here
//! instead of the unit-test module.

mod common;

use std::os::unix::net::UnixListener;

use common::unique_temp_dir;
use hyprlay_core::ctl::SocketProbe;
use hyprlay_core::ctl::probe_socket;

#[test]
fn live_listener_makes_the_guard_report_running() {
    let dir = unique_temp_dir("live");
    let path = dir.join("hyprlay.sock");
    let listener = UnixListener::bind(&path).unwrap();

    assert_eq!(probe_socket(&path), SocketProbe::AlreadyRunning);
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

    assert_eq!(probe_socket(&path), SocketProbe::StaleRemoved);
    assert!(!path.exists());
    // And the freed path accepts a fresh bind.
    let rebound = UnixListener::bind(&path);
    assert!(rebound.is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn absent_socket_reports_free() {
    let dir = unique_temp_dir("absent");
    assert_eq!(probe_socket(&dir.join("hyprlay.sock")), SocketProbe::Free);
    std::fs::remove_dir_all(&dir).unwrap();
}
