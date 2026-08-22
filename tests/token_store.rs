//! Integration tests for OAuth token persistence: save/load roundtrips,
//! issuer binding, and discard-on-mismatch all read and write real files
//! under a temp dir.

mod common;

use common::unique_temp_dir;
use hyprlay::daemon::adapters::token;
use serde_json::Value;

const OWN_APP_ID: &str = "123456789012345678";
const OTHER_APP_ID: &str = "999888777666555444";

#[test]
fn save_then_load_roundtrips_token_for_the_issuing_app() {
    let dir = unique_temp_dir("roundtrip");
    let path = dir.join("token.json");

    token::save_to(&path, "token-value-1", OWN_APP_ID);

    assert_eq!(
        token::load_from(&path, OWN_APP_ID),
        Some("token-value-1".to_string())
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn saved_tokens_record_the_issuing_application_id() {
    let dir = unique_temp_dir("recorded-issuer");
    let path = dir.join("token.json");

    token::save_to(&path, "token-value-2", OTHER_APP_ID);

    let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(v["client_id"], OTHER_APP_ID);
    assert_eq!(v["access_token"], "token-value-2");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn client_idless_token_file_is_discarded() {
    // Files written before tokens were app-bound carry no issuer; they
    // cannot be trusted for ANY application, so they are deleted.
    let dir = unique_temp_dir("client-idless");
    let path = dir.join("token.json");
    std::fs::write(&path, r#"{"access_token": "bindingless-token"}"#).unwrap();

    assert_eq!(token::load_from(&path, OWN_APP_ID), None);
    assert!(!path.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn modern_token_from_another_app_is_discarded_and_deleted() {
    let dir = unique_temp_dir("modern-other-app");
    let path = dir.join("token.json");
    token::save_to(&path, "foreign-token", OTHER_APP_ID);

    assert_eq!(token::load_from(&path, OWN_APP_ID), None);
    assert!(!path.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn corrupt_token_file_loads_as_none() {
    let dir = unique_temp_dir("corrupt");
    let path = dir.join("token.json");
    std::fs::write(&path, "not json at all").unwrap();

    assert_eq!(token::load_from(&path, OWN_APP_ID), None);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn missing_token_file_loads_as_none() {
    let dir = unique_temp_dir("missing");
    assert_eq!(token::load_from(&dir.join("absent.json"), OWN_APP_ID), None);
    std::fs::remove_dir_all(&dir).unwrap();
}
