use super::{body, load, save, scrub, Doorbell, FakeDoorbell, OnCall};
use crate::tempdir::TempDir;
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::Ordering;

#[test]
fn the_webhook_body_is_token_title_and_message_as_json() {
    let body: Value = serde_json::from_str(&body("tok", "Question", "hx-1 asks")).unwrap();
    assert_eq!(
        body,
        json!({"token": "tok", "title": "Question", "message": "hx-1 asks"})
    );
}

#[test]
fn a_failed_ring_never_shows_the_token() {
    let err = scrub(
        "https://x/?t=tok-123: http status: 403 (tok-123)",
        "tok-123",
    );
    assert!(!err.contains("tok-123"), "{err}");
    assert_eq!(err, "https://x/?t=***: http status: 403 (***)");
    assert_eq!(scrub("http status: 500", ""), "http status: 500");
}

#[test]
fn the_fake_doorbell_records_rings_and_fails_when_told() {
    let fake = FakeDoorbell::default();
    assert_eq!(fake.ring("tok", "Question", "hx-1 asks"), Ok(()));
    fake.fail.store(true, Ordering::SeqCst);
    assert!(fake.ring("tok", "Done", "run ended").is_err());
    assert_eq!(
        *fake.rings.lock().unwrap(),
        [
            ("tok".into(), "Question".into(), "hx-1 asks".into()),
            ("tok".into(), "Done".into(), "run ended".into()),
        ]
    );
}

/// No environment variable set.
fn no_env(_: &str) -> String {
    String::new()
}

#[test]
fn settings_round_trip_through_a_config_json_readable_only_by_the_user() {
    let repo = TempDir::new();
    let on_call = OnCall {
        token: Some("tok-123".into()),
        minutes: 12,
    };
    save(repo.path(), &on_call).unwrap();
    let path = repo.path().join(".orqadence-local/config.json");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(repo.path().join(".orqadence-local/.gitignore").exists());
    let doc: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(doc, json!({"on_call": {"token": "tok-123", "minutes": 12}}));
    assert_eq!(load(repo.path(), &no_env), on_call);
}

/// A repo whose config.json has `on_call` as its On call object.
fn repo_with(on_call: serde_json::Value) -> TempDir {
    let repo = TempDir::new();
    let path = repo.path().join(".orqadence-local/config.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, json!({"on_call": on_call}).to_string()).unwrap();
    repo
}

#[test]
fn moshi_webhook_token_wins_over_the_files_token() {
    let repo = repo_with(json!({"token": "tok-file", "minutes": 7}));
    let env = |key: &str| match key {
        "MOSHI_WEBHOOK_TOKEN" => " tok-env ".to_string(),
        _ => String::new(),
    };
    let on_call = load(repo.path(), &env);
    assert_eq!(on_call.token.as_deref(), Some("tok-env"));
    assert_eq!(on_call.minutes, 7);
}

#[test]
fn no_token_anywhere_is_off() {
    let bare = TempDir::new();
    assert_eq!(load(bare.path(), &no_env), OnCall::default());
    assert_eq!(OnCall::default().token, None);
    for on_call in [
        json!({"minutes": 3}),
        json!({"token": "  "}),
        json!({"token": 5}),
    ] {
        let repo = repo_with(on_call);
        assert_eq!(load(repo.path(), &no_env).token, None);
    }
}

#[test]
fn minutes_missing_zero_or_not_a_number_is_five() {
    for minutes in [
        None,
        Some(json!(0)),
        Some(json!("ten")),
        Some(json!(-3)),
        Some(json!(2.5)),
    ] {
        let on_call = match &minutes {
            Some(m) => json!({"token": "tok", "minutes": m}),
            None => json!({"token": "tok"}),
        };
        let repo = repo_with(on_call);
        assert_eq!(load(repo.path(), &no_env).minutes, 5, "minutes {minutes:?}");
    }
}
