//! Sibling-binary resolution against real directories. These tests touch
//! the filesystem, so they live in the integration suite (ticket 02 test
//! hygiene) instead of inside `src/dispatch.rs`.

mod common;

use common::unique_temp_dir;
use hyprlay::cli::dispatch::resolve_sibling;

#[test]
fn existing_sibling_resolves_to_its_full_path() {
    let dir = unique_temp_dir("present");
    std::fs::write(dir.join("hyprlayd"), b"").unwrap();

    let resolved = resolve_sibling(&dir, "hyprlayd");

    assert_eq!(resolved, Ok(dir.join("hyprlayd")));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn missing_sibling_names_the_binary_and_its_expected_location() {
    // A partial install (daemon missing) must produce an error that says
    // which file is missing and where it was looked for.
    let dir = unique_temp_dir("missing");
    let expected = dir.join("hyprlayd");

    let err = resolve_sibling(&dir, "hyprlayd").unwrap_err();

    assert!(err.contains("hyprlayd"), "error names the binary: {err}");
    assert!(
        err.contains(&expected.display().to_string()),
        "error shows the expected path: {err}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
