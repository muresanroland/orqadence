//! The Shell's graphify pass at open and every period, over fake Tools.

use super::shell_test::{await_line, line, screen_at};
use super::Screen;
use crate::orchestrator::app::{set_switch, GRAPHIFY};
use crate::orchestrator::world::wait_until;
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;
use crate::update::EVERY;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// A repo with graphify switched on, and fake Tools that record each
/// call's dir, tag v1.3.0 on origin's default branch, and fail `fails`.
fn graphify_on(fails: &'static str) -> (TempDir, Arc<Fake>, Arc<Mutex<Vec<PathBuf>>>) {
    let repo = TempDir::new();
    set_switch(repo.path(), &GRAPHIFY, true).unwrap();
    let dirs = Arc::new(Mutex::new(Vec::new()));
    let seen = dirs.clone();
    let fake = Fake::new(move |dir, argv| {
        seen.lock().unwrap().push(dir.to_path_buf());
        match argv.join(" ") {
            call if call == fails => Err("offline".to_string()),
            call if call == "uv tool list" => Ok("graphifyy v0.4.1\n- graphify\n".to_string()),
            call if call.starts_with("git tag --merged") => Ok("v1.2.0\nv1.3.0\n".to_string()),
            _ => Ok(String::new()),
        }
    });
    (repo, fake, dirs)
}

/// Polls until a pass has handed over its tag.
fn await_tag(s: &mut Screen) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.docs_tag.is_none() {
        assert!(Instant::now() < deadline, "no pass handed over a tag");
        s.poll();
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_pass_refreshes_the_graph_upgrades_graphify_and_finds_a_new_tag() {
    let (repo, fake, dirs) = graphify_on("");
    let mut s = screen_at(fake.clone(), repo.path());
    s.refresh_graphify(EVERY);
    await_tag(&mut s);
    assert_eq!(s.docs_tag.as_deref(), Some("v1.3.0"));
    let calls = fake.calls();
    for want in [
        "graphify update .",
        "uv tool upgrade graphifyy",
        "graphify install --platform claude",
        "graphify install --platform codex",
        "git fetch origin --tags",
        "git tag --merged origin/HEAD --list v*",
    ] {
        assert!(calls.iter().any(|c| c == want), "no {want:?} in {calls:?}");
    }
    assert!(dirs.lock().unwrap().iter().all(|d| d == repo.path()));
    assert!(s.events.is_empty(), "a pass that worked said something");
}

#[test]
fn with_graphify_off_no_pass_runs() {
    let repo = TempDir::new();
    let fake = Fake::quiet();
    let mut s = screen_at(fake.clone(), repo.path());
    s.refresh_graphify(Duration::from_millis(1));
    thread::sleep(Duration::from_millis(20));
    s.poll();
    assert_eq!(fake.calls(), Vec::<String>::new());
}

#[test]
fn a_failed_fetch_is_one_recent_line_and_the_rest_of_the_pass_ran() {
    let (repo, fake, _) = graphify_on("git fetch origin --tags");
    let mut s = screen_at(fake.clone(), repo.path());
    s.refresh_graphify(EVERY);
    await_line(
        &mut s,
        "graphify check failed: git fetch origin --tags: exit status 1: offline",
    );
    await_tag(&mut s);
    assert_eq!(
        s.events.len(),
        1,
        "{:?}",
        s.events.iter().map(line).collect::<Vec<_>>()
    );
    let calls = fake.calls();
    assert!(
        calls.contains(&"graphify update .".to_string()),
        "{calls:?}"
    );
    assert!(
        calls.contains(&"uv tool upgrade graphifyy".to_string()),
        "{calls:?}"
    );
}

#[test]
fn a_second_pass_comes_after_the_period() {
    let (repo, fake, _) = graphify_on("");
    let s = screen_at(fake.clone(), repo.path());
    s.refresh_graphify(Duration::from_millis(1));
    wait_until("a second pass", || {
        fake.calls()
            .iter()
            .filter(|c| *c == "graphify update .")
            .count()
            >= 2
    });
}

#[test]
fn a_pass_that_cannot_read_the_tags_keeps_the_tag_found_before() {
    let (repo, fake, _) = graphify_on("");
    let mut s = screen_at(fake.clone(), repo.path());
    s.refresh_graphify(EVERY);
    await_tag(&mut s);
    // a later pass, offline: the read and set-head both fail
    let offline = Fake::new(|_, argv| match argv[0] {
        "git" => Err("offline".to_string()),
        _ => Ok(String::new()),
    });
    s.cfg.tools = offline.clone();
    s.refresh_graphify(EVERY);
    await_line(
        &mut s,
        "graphify check failed: git remote set-head origin --auto",
    );
    assert_eq!(s.docs_tag.as_deref(), Some("v1.3.0"));
}

#[test]
fn a_switch_turned_off_since_open_runs_no_more_passes() {
    let (repo, fake, _) = graphify_on("");
    let s = screen_at(fake.clone(), repo.path());
    s.refresh_graphify(Duration::from_millis(1));
    let updates = || {
        fake.calls()
            .iter()
            .filter(|c| *c == "graphify update .")
            .count()
    };
    wait_until("a first pass", || updates() >= 1);
    set_switch(repo.path(), &GRAPHIFY, false).unwrap();
    thread::sleep(Duration::from_millis(20)); // a pass under way ends
    let after = updates();
    thread::sleep(Duration::from_millis(20));
    assert_eq!(updates(), after, "a pass ran with the switch off");
}
