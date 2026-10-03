use super::fake::Fake;
use super::{Exec, Tools};
use crate::tempdir::TempDir;
use std::path::Path;

#[test]
fn tools_seam_through_fake() {
    let fake = Fake::new(|_, argv| match argv.join(" ").as_str() {
        "git remote" => Ok("origin\n".to_string()),
        _ => Err("boom".to_string()),
    });
    let tools: &dyn Tools = &*fake;
    assert_eq!(
        tools.run(Path::new("/repo"), &["git", "remote"]).as_deref(),
        Ok("origin\n")
    );
    assert!(
        tools
            .run(Path::new("/repo"), &["gh", "auth", "status"])
            .is_err(),
        "want error from gh"
    );
    assert_eq!(fake.calls(), ["git remote", "gh auth status"]);
}

#[test]
fn exec_returns_stdout_and_stderr_in_error() {
    let dir = TempDir::new();
    let err = Exec
        .run(dir.path(), &["sh", "-c", "echo hi; echo oops >&2; exit 3"])
        .unwrap_err();
    assert!(
        err.to_string().contains("oops"),
        "err = {err}, want it to carry stderr"
    );
    assert_eq!(
        err.to_string(),
        "sh -c echo hi; echo oops >&2; exit 3: exit status 3: oops"
    );
}

#[test]
fn a_failure_redacts_the_typesafe_key_from_its_command_line() {
    let argv = ["sh", "-c", "exit 1", "--env", "TYPESAFE_API_KEY=sk-secret"];
    let err = Exec.run(Path::new("/"), &argv).unwrap_err();
    assert_eq!(
        err.to_string(),
        "sh -c exit 1 --env TYPESAFE_API_KEY=***: exit status 1: "
    );

    let fake = Fake::new(|_, _| Err("boom".to_string()));
    let err = fake.run(Path::new("/"), &argv).unwrap_err();
    assert!(!err.to_string().contains("sk-secret"), "{err}");
    assert_eq!(
        fake.calls(),
        [argv.join(" ")],
        "the recorded call keeps the argv"
    );
}

#[test]
fn run_within_kills_a_command_that_outlasts_its_limit() {
    let dir = TempDir::new();
    let started = std::time::Instant::now();
    let err = Exec
        .run_within(
            dir.path(),
            &["sleep", "5"],
            std::time::Duration::from_millis(100),
        )
        .unwrap_err();
    assert!(started.elapsed().as_secs() < 4, "it waited for sleep");
    assert_eq!(err.to_string(), "sleep 5: timed out after 0.1s: ");

    let out = Exec.run_within(
        dir.path(),
        &["echo", "hi"],
        std::time::Duration::from_secs(5),
    );
    assert_eq!(out.as_deref(), Ok("hi\n"));
}

#[test]
fn run_within_is_not_held_by_a_descendant_holding_the_pipes() {
    let dir = TempDir::new();
    let started = std::time::Instant::now();
    let out = Exec.run_within(
        dir.path(),
        &["sh", "-c", "sleep 30 & echo hi"],
        std::time::Duration::from_secs(5),
    );
    assert_eq!(out.as_deref(), Ok("hi\n"));
    assert!(started.elapsed().as_secs() < 4, "it waited for the sleep");

    let started = std::time::Instant::now();
    let err = Exec
        .run_within(
            dir.path(),
            &["sh", "-c", "sleep 30 & sleep 30"],
            std::time::Duration::from_millis(100),
        )
        .unwrap_err();
    assert!(started.elapsed().as_secs() < 4, "it waited for the sleeps");
    assert!(err.status.starts_with("timed out"), "{err}");
}

/// The editor: VISUAL, then EDITOR, an empty one unset and a bare code or
/// subl given its wait flag; else the first of code -w, vi, nano on PATH.
#[test]
fn the_editor_is_visual_then_editor_then_the_first_fallback_on_path() {
    let (a, b) = (TempDir::new(), TempDir::new());
    std::fs::write(b.path().join("nano"), "").unwrap();
    let path = format!("{}:{}", a.path().display(), b.path().display());
    let pick = |visual: &str, editor: &str| {
        super::editor(&|k| match k {
            "VISUAL" => visual.to_string(),
            "EDITOR" => editor.to_string(),
            "PATH" => path.clone(),
            _ => String::new(),
        })
    };
    assert_eq!(pick("hx", "emacs").as_deref(), Some("hx"));
    assert_eq!(pick("", "emacs -nw").as_deref(), Some("emacs -nw"));
    assert_eq!(pick(" ", "code").as_deref(), Some("code -w"));
    assert_eq!(pick("subl", "").as_deref(), Some("subl -w"));
    assert_eq!(pick("", "").as_deref(), Some("nano"));
    std::fs::write(a.path().join("vi"), "").unwrap();
    assert_eq!(pick("", "").as_deref(), Some("vi"));
    std::fs::write(b.path().join("code"), "").unwrap();
    assert_eq!(pick("", "").as_deref(), Some("code -w"));
    let none = super::editor(&|_| String::new());
    assert_eq!(none, None);
}
