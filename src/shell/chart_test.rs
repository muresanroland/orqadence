//! The charting session over the fake world: Start runs brainstorm-chart
//! beside the Shell, and its result closes the Idea with a Map or Tickets.

use crossterm::event::KeyCode;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use super::shell_test::{await_line, key, line, shell, type_in, type_line};
use super::Screen;
use crate::brainstorm::{Brainstorm, Phase, EPIC, GRILLING, IDEA, MAP};
use crate::orchestrator::herdr::PaneInfo;
use crate::orchestrator::state::LOCAL;
use crate::orchestrator::world::{new_world, BdTicket, Prompt, World};
use crate::tools::Tools;

/// The Idea Start creates, in bd before it as the fake world needs.
fn idea() -> BdTicket {
    BdTicket {
        labels: vec![IDEA.to_string()],
        no_epic: true,
        ..BdTicket::new("hx-7")
    }
}

/// A Waypoint of Map hx-m.
pub(super) fn waypoint(id: &str, label: &str) -> BdTicket {
    BdTicket {
        labels: vec![label.to_string()],
        parent: "hx-m".to_string(),
        ..BdTicket::new(id)
    }
}

/// Map hx-m with five grilling Waypoints and `epics` build-Epic ones.
pub(super) fn map(epics: usize) -> Vec<BdTicket> {
    let mut issues = vec![BdTicket {
        labels: vec![MAP.to_string()],
        no_epic: true,
        ..BdTicket::new("hx-m")
    }];
    for n in 1..=5 {
        issues.push(waypoint(&format!("hx-m.{n}"), GRILLING));
    }
    for n in 1..=epics {
        issues.push(waypoint(&format!("hx-m.e{n}"), EPIC));
    }
    issues
}

/// The world with `issues` beside the Idea, its Shell in pane w1:shell of
/// tab w1:t0, a Ticket label configured, and charting `session`.
pub(super) fn world(
    issues: Vec<BdTicket>,
    session: impl Fn(&Prompt) -> (String, String) + Send + Sync + 'static,
) -> Arc<World> {
    let mut all = vec![idea()];
    all.extend(issues);
    let (w, _) = new_world(all);
    {
        let mut inner = w.lock();
        inner.tabs.push("w1:t0".to_string());
        inner.panes.push(PaneInfo {
            pane_id: "w1:shell".to_string(),
            tab_id: "w1:t0".to_string(),
        });
        if let Some(m) = inner.tickets.iter_mut().find(|t| t.id == "hx-m") {
            m.issue_type = "epic".to_string();
        }
    }
    let config = w.repo.join(".orqadence/config.json");
    let mut doc: serde_json::Value = fs::read(&config)
        .ok()
        .and_then(|raw| serde_json::from_slice(&raw).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    doc["labels"] = serde_json::json!({"fe": {"kind": "area", "guidance": "Screens and styles"}});
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(&config, doc.to_string()).unwrap();
    w.hook(|_, argv| (argv.starts_with(&["bd", "create"])).then(|| Ok("hx-7\n".to_string())));
    w.session(session);
    w
}

/// The Shell after Start on an idea, its charting session started.
pub(super) fn started(w: &Arc<World>) -> Screen {
    let mut s = shell(w);
    s.shell_pane = "w1:shell".to_string();
    type_line(&mut s, "/brainstorm");
    type_in(&mut s, "Queue bd writes");
    s.key(key(KeyCode::Enter));
    await_line(&mut s, "charting started");
    s
}

/// A charting session that writes `result` and goes idle.
pub(super) fn writes(result: &'static str) -> impl Fn(&Prompt) -> (String, String) + Send + Sync {
    move |p: &Prompt| match p.stage.as_str() {
        "chart" => (result.to_string(), "idle".to_string()),
        _ => (String::new(), "idle".to_string()),
    }
}

pub(super) fn idle(_: &Prompt) -> (String, String) {
    (String::new(), "idle".to_string())
}

fn worktree(w: &World) -> PathBuf {
    w.repo.join(LOCAL).join("worktrees/hx-7")
}

pub(super) fn saved(w: &World) -> Brainstorm {
    let file = w.repo.join(LOCAL).join("brainstorms/hx-7/state.json");
    serde_json::from_slice(&fs::read(file).unwrap()).unwrap()
}

fn status(w: &World, id: &str) -> (String, String) {
    let inner = w.lock();
    let t = inner.tickets.iter().find(|t| t.id == id).unwrap();
    (t.status.clone(), t.close_reason.clone())
}

/// The prompt herdr took that contains `needle`, waited for up to 5s.
pub(super) fn await_prompt(w: &World, needle: &str) -> String {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let sent = w.called("herdr agent prompt");
        if let Some(p) = sent.into_iter().find(|p| p.contains(needle)) {
            return p;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no prompt contains {needle}"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

pub(super) fn pane_alive(w: &World, pane: &str) -> bool {
    w.lock().panes.iter().any(|p| p.pane_id == pane)
}

/// Polls the Shell for a while, as its run loop does.
fn wait_a_while(s: &mut Screen) {
    for _ in 0..40 {
        s.poll();
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn start_runs_brainstorm_chart_split_from_the_shells_pane() {
    let w = world(Vec::new(), idle);
    let mut s = started(&w);

    let tree = worktree(&w).display().to_string();
    let split = w.called("herdr pane split");
    assert_eq!(split.len(), 1, "{split:?}");
    assert!(
        split[0].starts_with("herdr pane split w1:shell ")
            && split[0].contains(" --ratio ")
            && split[0].contains(&format!("--cwd {tree}")),
        "{split:?}"
    );
    let result = w.repo.join(LOCAL).join("brainstorms/hx-7/chart.md");
    let prompt = w.prompt(&result.display().to_string());
    for want in [
        "# Brainstorm charting",
        "- IDEA: hx-7\n",
        "- TICKET LABELS: orqa:fe (area): Screens and styles\n",
        &format!("- RESULT FILE: {}\n", result.display()),
    ] {
        assert!(prompt.contains(want), "{want:?} not in:\n{prompt}");
    }
    assert!(!prompt.contains("{{prose}}"), "{prompt}");
    assert!(
        s.events
            .iter()
            .any(|e| line(e) == "hx-7 charting started: claude (pane 1-2)"),
        "{:?}",
        s.events.iter().map(line).collect::<Vec<_>>()
    );
    s.close();
}

#[test]
fn an_idle_pane_with_no_result_file_changes_nothing_and_asks_nothing() {
    let w = world(Vec::new(), idle);
    let mut s = started(&w);
    let pane = saved(&w).pane;

    wait_a_while(&mut s);

    assert!(w.called("bd close").is_empty());
    assert!(pane_alive(&w, &pane));
    assert!(s.questions.is_empty());
    assert!(line(s.events.last().unwrap()).contains("charting started"));
    assert_eq!(saved(&w).phase, Phase::Charting);
    s.close();
}

#[test]
fn close_joins_the_driver_and_leaves_its_pane_running() {
    let w = world(Vec::new(), idle);
    let mut s = started(&w);
    let pane = saved(&w).pane;

    s.close();

    assert!(s.brainstorm_threads.is_empty());
    let watched = w.called("herdr agent get").len();
    thread::sleep(Duration::from_millis(20));
    assert_eq!(
        w.called("herdr agent get").len(),
        watched,
        "the driver still runs"
    );
    assert!(pane_alive(&w, &pane));
    assert_eq!(saved(&w).phase, Phase::Charting);
}

#[test]
fn a_map_with_one_build_epic_waypoint_closes_the_idea_and_the_pane_and_keeps_the_worktree() {
    let w = world(
        map(1),
        writes("STATUS: done\nMAP: hx-m\n\nthe Destination\n"),
    );
    let mut s = started(&w);
    let pane = saved(&w).pane;

    await_line(
        &mut s,
        "hx-7 charting done: Map hx-m, 6 Waypoints; Idea closed",
    );

    assert_eq!(
        status(&w, "hx-7"),
        ("closed".to_string(), "Map hx-m".to_string())
    );
    assert!(!pane_alive(&w, &pane));
    assert!(worktree(&w).exists(), "the Map keeps its worktree");
    let b = saved(&w);
    assert_eq!((b.phase.clone(), b.map.as_str()), (Phase::Map, "hx-m"));
    assert!(b.pane.is_empty());
    assert_eq!(s.brainstorms, [b]);
    s.close();
}

#[test]
fn a_pane_close_failing_otherwise_than_not_found_keeps_the_pane_id() {
    let w = world(
        map(1),
        writes("STATUS: done\nMAP: hx-m\n\nthe Destination\n"),
    );
    w.fail_once("herdr pane close", "herdr: timeout");
    let mut s = started(&w);
    let pane = saved(&w).pane;

    await_line(
        &mut s,
        "hx-7 charting done: Map hx-m, 6 Waypoints; Idea closed",
    );

    assert!(pane_alive(&w, &pane));
    assert!(
        s.events
            .iter()
            .any(|e| line(e).contains("charting pane not closed: ")
                && line(e).contains("herdr: timeout")),
        "{:?}",
        s.events.iter().map(line).collect::<Vec<_>>()
    );
    assert_eq!(saved(&w).pane, pane);
    s.close();
}

#[test]
fn a_map_with_two_build_epic_waypoints_leaves_the_idea_and_the_pane_open() {
    let w = world(map(2), writes("STATUS: done\nMAP: hx-m\n"));
    let mut s = started(&w);
    let pane = saved(&w).pane;

    await_line(
        &mut s,
        "hx-7 charting result not taken: Map hx-m has 2 open brainstorm:epic Waypoints, not one",
    );
    wait_a_while(&mut s);

    assert_eq!(status(&w, "hx-7").0, "in_progress");
    assert!(pane_alive(&w, &pane));
    assert_eq!(saved(&w).phase, Phase::Charting);
    let said = s.events.iter().filter(|e| e.text.contains("not taken"));
    assert_eq!(said.count(), 1, "said once, not every tick");
    s.close();
}

#[test]
fn tickets_close_the_idea_and_remove_the_worktree_keeping_its_branch() {
    let tickets = vec![BdTicket::new("hx-1"), BdTicket::new("hx-2")];
    let w = world(
        tickets,
        writes("STATUS: done\nTICKETS: hx-1 hx-2\nPR: https://example.test/pr/4\nLABEL: docs | Docs | skills: | tickets: hx-2\n"),
    );
    let mut s = started(&w);
    let pane = saved(&w).pane;

    await_line(
        &mut s,
        "hx-7 charting done: Tickets hx-1, hx-2; Idea closed; docs PR https://example.test/pr/4",
    );

    assert_eq!(
        status(&w, "hx-7"),
        ("closed".to_string(), "Tickets hx-1, hx-2".to_string())
    );
    assert!(!pane_alive(&w, &pane));
    assert!(!worktree(&w).exists(), "the worktree is removed");
    assert_eq!(w.called("git worktree remove").len(), 1);
    assert!(w.called("git branch").is_empty(), "its branch is kept");
    let b = saved(&w);
    assert_eq!(b.phase, Phase::Done);
    assert_eq!(b.branch, "brainstorm/hx-7");
    assert_eq!(b.label_lines, ["docs | Docs | skills: | tickets: hx-2"]);
    s.close();
}

#[test]
fn the_sessions_app_and_id_are_in_the_brainstorms_state_never_in_state_json() {
    let w = world(Vec::new(), idle);
    w.lock().integration = true;
    let mut s = started(&w);

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while saved(&w).session.is_none_or(|s| s.id.is_empty()) {
        assert!(std::time::Instant::now() < deadline, "no session id saved");
        s.poll();
        thread::sleep(Duration::from_millis(1));
    }

    let session = saved(&w).session.unwrap();
    assert_eq!(session.app, "claude");
    let pane = saved(&w).pane;
    assert_eq!(w.lock().sessions.get(&pane), Some(&session.id));
    let state = fs::read_to_string(w.repo.join(LOCAL).join("state.json")).unwrap_or_default();
    assert!(!state.contains(&session.id), "{state}");
    s.close();
}

#[test]
fn the_pane_closed_by_the_user_stops_the_brainstorm_saved() {
    let w = world(Vec::new(), idle);
    let mut s = started(&w);
    let pane = saved(&w).pane;

    w.run(&w.repo, &["herdr", "pane", "close", &pane]).unwrap();
    await_line(
        &mut s,
        "hx-7 charting stopped: its pane is gone; Brainstorm saved",
    );

    let b = saved(&w);
    assert_eq!(b.phase, Phase::Charting);
    assert!(b.pane.is_empty());
    assert_eq!(b.session.map(|s| s.app).as_deref(), Some("claude"));
    assert_eq!(status(&w, "hx-7").0, "in_progress");
    assert!(worktree(&w).exists());
    s.close();
}

#[test]
fn an_agent_get_failing_with_the_pane_still_there_keeps_watching() {
    let w = world(vec![BdTicket::new("hx-1")], idle);
    let mut s = started(&w);
    let pane = saved(&w).pane;
    let before = w.calls().len();

    w.fail_once("herdr agent get", "herdr: timeout");
    for _ in 0..50 {
        if w.since(before, "herdr agent get").len() >= 2 {
            break;
        }
        wait_a_while(&mut s);
    }

    assert!(!s.events.iter().any(|e| e.text.contains("charting stopped")));
    assert_eq!(saved(&w).pane, pane);
    let result = w.repo.join(LOCAL).join("brainstorms/hx-7/chart.md");
    fs::write(result, "STATUS: done\nTICKETS: hx-1\n").unwrap();
    await_line(&mut s, "hx-7 charting done: Tickets hx-1; Idea closed");
    s.close();
}

#[test]
fn a_result_written_as_the_session_exits_is_still_taken() {
    let w = world(vec![BdTicket::new("hx-1")], idle);
    let world = Arc::downgrade(&w);
    w.session(move |p: &Prompt| {
        // the session writes its result and its pane closes with it
        if let Some(w) = world.upgrade() {
            w.lock().agents.remove(&p.pane);
        }
        (
            "STATUS: done\nTICKETS: hx-1\n".to_string(),
            "idle".to_string(),
        )
    });
    let mut s = started(&w);

    await_line(&mut s, "hx-7 charting done: Tickets hx-1; Idea closed");

    assert!(!s.events.iter().any(|e| e.text.contains("charting stopped")));
    s.close();
}

#[test]
fn a_result_ready_on_the_first_tick_still_saves_the_session_id() {
    let w = world(
        vec![BdTicket::new("hx-1")],
        writes("STATUS: done\nTICKETS: hx-1\n"),
    );
    w.lock().integration = true;
    let mut s = started(&w);

    await_line(&mut s, "hx-7 charting done: Tickets hx-1; Idea closed");

    let session = saved(&w).session.unwrap();
    assert!(!session.id.is_empty(), "no session id saved");
    s.close();
}

#[test]
fn the_pane_closed_during_the_trust_wait_stops_the_brainstorm_saved() {
    let w = world(Vec::new(), idle);
    let projects = format!(
        r#"{{"projects": {{"{}": {{"hasTrustDialogAccepted": false}}}}}}"#,
        worktree(&w).display()
    );
    fs::write(w.home.join(".claude.json"), projects).unwrap();
    let mut s = shell(&w);
    s.shell_pane = "w1:shell".to_string();
    type_line(&mut s, "/brainstorm");
    type_in(&mut s, "Queue bd writes");
    s.key(key(KeyCode::Enter));
    await_line(&mut s, "hx-7 waiting: claude does not trust");
    let pane = saved(&w).pane;

    w.run(&w.repo, &["herdr", "pane", "close", &pane]).unwrap();
    await_line(
        &mut s,
        "hx-7 charting stopped: its pane is gone; Brainstorm saved",
    );

    let b = saved(&w);
    assert_eq!(b.phase, Phase::Charting);
    assert!(b.pane.is_empty());
    s.close();
}
