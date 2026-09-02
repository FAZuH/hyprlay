//! Integration tests for Discord IPC socket discovery: each test plants
//! real socket files under a temp runtime dir and checks the probe order.
#![cfg(unix)]

mod common;

use common::unique_temp_dir;
use hyprlay::daemon::adapters::ipc::discover;

#[test]
fn discover_picks_the_first_existing_socket_in_an_injected_dir() {
    let dir = unique_temp_dir("first-existing");
    std::fs::write(dir.join("discord-ipc-2"), b"").unwrap();

    let found = discover(&dir).expect("socket exists");

    assert_eq!(found, dir.join("discord-ipc-2"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn discover_prefers_a_plain_socket_over_a_flatpak_one() {
    let dir = unique_temp_dir("plain-over-flatpak");
    let flatpak = dir.join("app/com.discordapp.Discord");
    std::fs::create_dir_all(&flatpak).unwrap();
    std::fs::write(flatpak.join("discord-ipc-0"), b"").unwrap();
    std::fs::write(dir.join("discord-ipc-5"), b"").unwrap();

    let found = discover(&dir).expect("sockets exist");

    assert_eq!(found, dir.join("discord-ipc-5"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn discover_returns_none_when_no_socket_exists() {
    let dir = unique_temp_dir("empty");
    assert_eq!(discover(&dir), None);
    std::fs::remove_dir_all(&dir).unwrap();
}
