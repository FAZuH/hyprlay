//! Integration tests for auth.json persistence: roundtrips, incomplete-pair
//! removal, and file permissions write real files under a temp dir.

mod common;

use common::unique_temp_dir;
use hyprlay_core::credentials::AppCredentials;
use hyprlay_core::credentials::load_from;
use hyprlay_core::credentials::save_to;

fn creds(id: &str, secret: &str) -> AppCredentials {
    AppCredentials {
        client_id: id.to_string(),
        client_secret: secret.to_string(),
    }
}

#[test]
fn credentials_roundtrip_through_save_and_load() {
    let dir = unique_temp_dir("roundtrip");
    let path = dir.join("auth.json");
    let original = creds("109876543210987654", "round-trip-secret");

    save_to(&path, &original).unwrap();

    assert_eq!(load_from(&path), Some(original));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn saving_incomplete_credentials_removes_an_existing_file() {
    let dir = unique_temp_dir("incomplete-save");
    let path = dir.join("auth.json");
    save_to(&path, &creds("some-id", "some-secret")).unwrap();
    assert!(path.exists());

    save_to(&path, &creds("some-id", "")).unwrap();

    assert!(!path.exists());
    // And removing again when nothing exists is still a success.
    save_to(&path, &creds("", "")).unwrap();
    assert!(!path.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn saved_credentials_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = unique_temp_dir("permissions");
    let path = dir.join("auth.json");

    save_to(&path, &creds("id-1", "secret-1")).unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    std::fs::remove_dir_all(&dir).unwrap();
}
