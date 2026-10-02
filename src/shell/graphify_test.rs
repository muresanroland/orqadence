//! The Shell's graphify pass at open and every period, and the Docs pass
//! Question and pass on the new tag it finds, over fake Tools.

use super::shell_test::{await_line, line, screen_at};
use super::Screen;
use crate::graphify::handled;
use crate::orchestrator::app::{read_object, set_switch, write, DOCS_PASS, GRAPHIFY};
use crate::orchestrator::state::STATUS_PARKED;
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

/// Polls until a pass has raised the Docs pass Question.
fn await_tag(s: &mut Screen) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.questions.is_empty() {
        assert!(
            Instant::now() < deadline,
            "no pass raised the Docs pass Question"
        );
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
    assert_eq!(
        s.questions[0].text,
        "v1.3.0 tagged: run the graphify docs pass now?"
    );
    let calls = fake.calls();
    for want in [
        "graphify update .",
        "uv tool upgrade graphifyy",
        "graphify install --platform claude",
        "graphify install --platform codex",
        "git fetch origin --tags",
        "git remote set-head origin --auto",
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

/// A checkout with graphify on whose checks read `tags`, and a herdr whose
/// `agent get` answers `gets` in turn, then idle: "working", "idle",
/// "built" (graph.json written, then idle) or "gone" (no agent).
struct Docs {
    repo: TempDir,
    fake: Arc<Fake>,
    tags: Arc<Mutex<&'static str>>,
}

/// The Docs pass's checkout and fake herdr, its checks reading v1.4.0.
fn docs(gets: &[&'static str]) -> Docs {
    let repo = TempDir::new();
    set_switch(repo.path(), &GRAPHIFY, true).unwrap();
    let tags = Arc::new(Mutex::new("v1.4.0"));
    let (read, graph) = (tags.clone(), repo.path().join("graphify-out"));
    let gets = Mutex::new(gets.to_vec());
    let fake = Fake::new(move |_, argv| {
        let reply = |result: &str| Ok(format!(r#"{{"result":{result}}}"#));
        match argv[..2.min(argv.len())].join(" ").as_str() {
            "git tag" => Ok(read.lock().unwrap().to_string()),
            "herdr tab" if argv[2] == "create" => {
                reply(r#"{"tab":{"tab_id":"t9"},"root_pane":{"pane_id":"p9","tab_id":"t9"}}"#)
            }
            "herdr tab" if argv[2] == "list" => reply(r#"{"tabs":[{"tab_id":"t9"}]}"#),
            "herdr pane" => reply(r#"{"panes":[{"pane_id":"p9","tab_id":"t9"}]}"#),
            "herdr agent" if argv[2] == "get" => {
                let mut gets = gets.lock().unwrap();
                let get = match gets.is_empty() {
                    true => "idle",
                    false => gets.remove(0),
                };
                let status = match get {
                    "gone" => return Err("no agent in pane p9".to_string()),
                    "built" => {
                        std::fs::create_dir_all(&graph).unwrap();
                        std::fs::write(graph.join("graph.json"), "{}").unwrap();
                        "idle"
                    }
                    status => status,
                };
                reply(&format!(
                    r#"{{"agent":{{"agent_status":"{status}","pane_id":"p9"}}}}"#
                ))
            }
            "herdr agent" => Ok("{}".to_string()),
            _ => Ok(String::new()),
        }
    });
    Docs { repo, fake, tags }
}

/// One check that reads `tag`, applied by poll() once it has run.
fn check(s: &mut Screen, d: &Docs, tag: &'static str) {
    *d.tags.lock().unwrap() = tag;
    let reads = || {
        let calls = d.fake.calls();
        calls
            .iter()
            .filter(|c| c.starts_with("git tag --merged"))
            .count()
    };
    let before = reads();
    s.refresh_graphify(EVERY);
    wait_until("a check", || reads() > before);
    thread::sleep(Duration::from_millis(20)); // its hand-over
    s.poll();
}

/// The herdr calls so far, in order.
fn herdr_calls(fake: &Fake) -> Vec<String> {
    fake.calls()
        .into_iter()
        .filter(|c| c.starts_with("herdr"))
        .collect()
}

/// A pass on v1.4.0 that recorded nothing.
const UNFINISHED: &str =
    "graphify docs pass on v1.4.0 did not finish: asked again at the next check";

#[test]
fn a_new_tag_asks_and_no_records_it_with_no_pass() {
    let d = docs(&[]);
    let mut s = screen_at(d.fake.clone(), d.repo.path());
    check(&mut s, &d, "v1.4.0");
    assert_eq!(s.questions.len(), 1);
    assert_eq!(
        s.questions[0].text,
        "v1.4.0 tagged: run the graphify docs pass now?"
    );
    assert_eq!(s.options(), ["yes", "no"]);
    s.command("n");
    assert!(s.questions.is_empty());
    assert_eq!(handled(d.repo.path()), Some((1, 4)));
    assert_eq!(herdr_calls(&d.fake), Vec::<String>::new());
    check(&mut s, &d, "v1.4.2");
    assert!(s.questions.is_empty(), "a patch tag asked");
    check(&mut s, &d, "v1.5.0");
    assert_eq!(
        s.questions[0].text,
        "v1.5.0 tagged: run the graphify docs pass now?"
    );
}

#[test]
fn yes_starts_the_pass_in_a_graphify_tab_and_a_newer_graph_records_it() {
    let d = docs(&["working", "built"]);
    let (path, mut doc) = read_object(d.repo.path()).unwrap();
    doc[DOCS_PASS] = serde_json::json!({ "model": "opus", "effort": "high" });
    write(&path, &doc).unwrap();
    let mut s = screen_at(d.fake.clone(), d.repo.path());
    check(&mut s, &d, "v1.4.0");
    s.command("y");
    await_line(
        &mut s,
        "graphify docs pass started on v1.4.0: claude opus/high (pane 1-1)",
    );
    await_line(&mut s, "graphify docs pass done on v1.4.0");
    let calls = herdr_calls(&d.fake);
    let at = |prefix: &str| {
        calls
            .iter()
            .position(|c| c.starts_with(prefix))
            .unwrap_or_else(|| panic!("no {prefix:?} in {calls:?}"))
    };
    let repo = d.repo.path().display().to_string();
    let create = &calls[at("herdr tab create")];
    for want in ["--label graphify", &format!("--cwd {repo}"), "--no-focus"] {
        assert!(create.contains(want), "no {want:?} in {create}");
    }
    let start = &calls[at("herdr agent start")];
    assert!(start.contains("--kind claude --pane p9 --"), "{start}");
    assert!(start.ends_with("--model opus --effort high"), "{start}");
    let prompt = at("herdr agent prompt p9 /graphify . --update");
    assert!(at("herdr tab create") < at("herdr agent start") && at("herdr agent start") < prompt);
    assert!(
        calls.contains(&"herdr tab close t9".to_string()),
        "{calls:?}"
    );
    assert_eq!(handled(d.repo.path()), Some((1, 4)));
    assert!(s.docs_tag.is_none(), "the pass is still live");
}

#[test]
fn on_codex_the_pass_is_prompted_with_its_mention() {
    let d = docs(&["working", "built"]);
    let (path, mut doc) = read_object(d.repo.path()).unwrap();
    doc[DOCS_PASS] = serde_json::json!({ "app": "codex" });
    write(&path, &doc).unwrap();
    let mut s = screen_at(d.fake.clone(), d.repo.path());
    check(&mut s, &d, "v1.4.0");
    s.command("y");
    await_line(&mut s, "graphify docs pass done on v1.4.0");
    let calls = herdr_calls(&d.fake);
    assert!(
        calls.iter().any(|c| c.contains("--kind codex")),
        "{calls:?}"
    );
    assert!(
        calls.contains(&"herdr agent prompt p9 $graphify . --update".to_string()),
        "{calls:?}"
    );
}

#[test]
fn the_pane_gone_mid_pass_records_nothing() {
    let d = docs(&["working", "gone"]);
    let mut s = screen_at(d.fake.clone(), d.repo.path());
    check(&mut s, &d, "v1.4.0");
    s.command("y");
    await_line(&mut s, UNFINISHED);
    assert_eq!(handled(d.repo.path()), None);
    assert!(s.docs_tag.is_none(), "the pass is still live");
}

#[test]
fn idle_with_no_newer_graph_records_nothing_and_leaves_the_tab_open() {
    let d = docs(&["working", "idle"]);
    let out = d.repo.path().join("graphify-out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("graph.json"), "{}").unwrap(); // the last pass's
    let mut s = screen_at(d.fake.clone(), d.repo.path());
    check(&mut s, &d, "v1.4.0");
    s.command("y");
    await_line(&mut s, UNFINISHED);
    assert_eq!(handled(d.repo.path()), None);
    let calls = herdr_calls(&d.fake);
    assert!(
        !calls.iter().any(|c| c.starts_with("herdr tab close")),
        "{calls:?}"
    );
}

#[test]
fn under_away_the_question_waits_and_nothing_parks() {
    let d = docs(&[]);
    let mut s = screen_at(d.fake.clone(), d.repo.path());
    s.command("/away");
    check(&mut s, &d, "v1.4.0");
    assert_eq!(s.questions.len(), 1);
    assert!(s.questions[0].ticket.is_none());
    assert!(!s
        .state
        .tickets
        .values()
        .any(|ts| ts.status == STATUS_PARKED));
}

#[test]
fn a_second_new_tag_while_the_question_is_open_asks_nothing() {
    let d = docs(&[]);
    let mut s = screen_at(d.fake.clone(), d.repo.path());
    check(&mut s, &d, "v1.4.0");
    check(&mut s, &d, "v1.5.0");
    assert_eq!(s.questions.len(), 1);
    assert_eq!(
        s.questions[0].text,
        "v1.4.0 tagged: run the graphify docs pass now?"
    );
}

#[test]
fn idle_without_ever_working_records_nothing_even_with_a_newer_graph() {
    let d = docs(&["idle", "idle", "idle", "built"]);
    let mut s = screen_at(d.fake.clone(), d.repo.path());
    check(&mut s, &d, "v1.4.0");
    s.command("y");
    await_line(&mut s, UNFINISHED);
    assert_eq!(handled(d.repo.path()), None);
}

#[test]
fn exit_mid_pass_closes_its_tab_and_records_nothing() {
    let d = docs(&["working"; 50]);
    let mut s = screen_at(d.fake.clone(), d.repo.path());
    check(&mut s, &d, "v1.4.0");
    s.command("y");
    await_line(
        &mut s,
        "graphify docs pass started on v1.4.0: claude (pane 1-1)",
    );
    s.command("/exit");
    assert!(s.quit);
    let calls = herdr_calls(&d.fake);
    assert!(
        calls.contains(&"herdr tab close t9".to_string()),
        "{calls:?}"
    );
    assert_eq!(handled(d.repo.path()), None);
}
