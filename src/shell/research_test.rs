//! Research in the background over the fake world: a live Map's Research
//! Waypoints start in tab research-<map>, max_research at once, each a
//! Stage of the Brainstorm's own Orchestrator with its Wake and Away.

use crossterm::event::KeyCode;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use super::chart_test::{charted, pane_alive, saved, started, waypoint, world};
use super::shell_test::{await_line, key};
use super::{About, Screen};
use crate::brainstorm::{EPIC, GRILLING, MAP, RESEARCH};
use crate::orchestrator::stage::Ask;
use crate::orchestrator::state::LOCAL;
use crate::orchestrator::world::{working, BdTicket, Prompt, World};
use crate::tools::Tools;

/// Charting writes Map hx-m; every other session works on.
fn session(p: &Prompt) -> (String, String) {
    match p.stage.as_str() {
        "chart" => ("STATUS: done\nMAP: hx-m\n".to_string(), "idle".to_string()),
        _ => working(p),
    }
}

/// Map hx-m with `n` Research Waypoints and its build-Epic one.
fn research(n: usize) -> Vec<BdTicket> {
    let mut issues = vec![BdTicket {
        labels: vec![MAP.to_string()],
        no_epic: true,
        ..BdTicket::new("hx-m")
    }];
    for i in 1..=n {
        issues.push(waypoint(&format!("hx-m.{i}"), RESEARCH));
    }
    issues.push(waypoint("hx-m.e1", EPIC));
    issues
}

/// Idea hx-7 charted into Map hx-m over `issues`, and Start Map pressed
/// with research in the background.
fn live(
    issues: Vec<BdTicket>,
    session: impl Fn(&Prompt) -> (String, String) + Send + Sync + 'static,
) -> (Arc<World>, Screen) {
    let w = world(issues, session);
    let mut s = started(&w);
    await_line(&mut s, "hx-7 charting done: Map hx-m");
    charted(&mut s);
    s.key(key(KeyCode::Enter));
    (w, s)
}

/// The place in the calls of the first one starting with `prefix`.
fn first(w: &World, prefix: &str) -> usize {
    let calls = w.calls();
    calls
        .iter()
        .position(|c| c.starts_with(prefix))
        .unwrap_or_else(|| panic!("no call {prefix:?} in:\n{}", calls.join("\n")))
}

#[test]
fn max_research_sessions_start_in_tab_research_map_the_rest_wait() {
    let (w, mut s) = live(research(3), session);

    await_line(&mut s, "hx-m.1 research started: claude");
    await_line(&mut s, "hx-m.2 research started: claude");
    await_line(&mut s, "hx-m.3 waits for a research slot, 2 of 2 running");

    for n in [1, 2] {
        let claim = first(&w, &format!("bd update hx-m.{n} --claim"));
        let start = first(&w, &format!("herdr agent start h-hx-m-{n}-research"));
        assert!(claim < start, "hx-m.{n} started before its claim");
    }
    let tabs = w.called("herdr tab create");
    assert_eq!(tabs.len(), 1, "{tabs:?}");
    assert!(tabs[0].contains("--label research-hx-m"), "{tabs:?}");
    assert!(w.called("herdr agent start h-hx-m-3-research").is_empty());
    assert!(w.called("bd update hx-m.3").is_empty());
    s.close();
}

/// The pane of `id`'s research session.
fn pane(w: &World, id: &str) -> String {
    let name = format!("h-{}-research", id.replace('.', "-"));
    w.lock().names[&name].clone()
}

/// `id`'s research session writes its result, closing its Waypoint in bd
/// when `close`, and goes idle.
fn finish(w: &World, id: &str, close: bool) {
    if close {
        w.run(&w.repo, &["bd", "close", id, "--reason", "found"])
            .unwrap();
    }
    let file = w.repo.join(LOCAL).join("runs").join(id).join("research.md");
    std::fs::write(file, format!("STATUS: done\nWAYPOINT: {id}\n")).unwrap();
    let pane = pane(w, id);
    w.lock().agents.insert(pane, "idle".to_string());
}

#[test]
fn a_close_frees_its_slot_and_the_waiting_one_starts() {
    let (w, mut s) = live(research(3), session);
    await_line(&mut s, "hx-m.3 waits for a research slot");
    await_line(&mut s, "hx-m.1 research started");
    let first = pane(&w, "hx-m.1");

    finish(&w, "hx-m.1", true);

    await_line(&mut s, "hx-m.1 closed; its pane closes");
    await_line(&mut s, "hx-m.3 research started");
    assert!(!pane_alive(&w, &first));
    let saved = saved(&w).research;
    let held: Vec<&str> = saved.iter().map(|r| r.waypoint.as_str()).collect();
    assert_eq!(held, ["hx-m.2", "hx-m.3"]);
    s.close();
}

#[test]
fn a_result_with_its_waypoint_open_keeps_the_pane_and_the_slot() {
    let (w, mut s) = live(research(3), session);
    await_line(&mut s, "hx-m.3 waits for a research slot");
    await_line(&mut s, "hx-m.1 research started");
    let first = pane(&w, "hx-m.1");

    finish(&w, "hx-m.1", false);

    await_line(
        &mut s,
        "hx-m.1 result written, but 1 is still open in bd: pane",
    );
    assert!(pane_alive(&w, &first));
    assert!(w.called("herdr agent start h-hx-m-3-research").is_empty());
    w.run(&w.repo, &["bd", "close", "hx-m.1"]).unwrap();
    await_line(&mut s, "hx-m.1 closed; its pane closes");
    await_line(&mut s, "hx-m.3 research started");
    s.close();
}

#[test]
fn a_research_waypoint_created_later_starts_on_the_frontier() {
    let (w, mut s) = live(research(1), session);
    await_line(&mut s, "hx-m.1 research started");

    w.lock().tickets.push(BdTicket {
        status: "open".to_string(),
        issue_type: "task".to_string(),
        ..waypoint("hx-m.9", RESEARCH)
    });

    await_line(&mut s, "hx-m.9 research started");
    s.close();
}

/// Research Waypoints at once, in config.json.
fn max_research(w: &World, n: usize) {
    let config = w.repo.join(".orqadence/config.json");
    let mut doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    doc["max_research"] = serde_json::json!(n);
    std::fs::write(&config, doc.to_string()).unwrap();
}

/// Charting writes Map hx-m; every other session idles with no result.
fn idles(p: &Prompt) -> (String, String) {
    match p.stage.as_str() {
        "chart" => session(p),
        _ => (String::new(), "idle".to_string()),
    }
}

/// Polls the Shell until a Question about `id` waits.
fn await_question(s: &mut Screen, id: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !s.questions.iter().any(|q| q.ticket.as_deref() == Some(id)) {
        assert!(Instant::now() < deadline, "no Question about {id}");
        s.poll();
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn an_idle_research_pane_with_no_result_wakes() {
    let (_w, mut s) = live(research(1), idles);

    await_line(
        &mut s,
        "hx-m.1 stuck in research: went idle without a result (pane",
    );
    await_question(&mut s, "hx-m.1");

    let q = s
        .questions
        .iter()
        .find(|q| q.ticket.as_deref() == Some("hx-m.1"));
    assert!(matches!(q.unwrap().about, About::Asked(Ask::Wake { .. })));
    s.close();
}

/// Research hx-m.1 parked under Away, its slot of one taken by hx-m.2.
fn parked() -> (Arc<World>, Screen, String) {
    let w = world(research(2), idles);
    w.lock().integration = true;
    max_research(&w, 1);
    let mut s = started(&w);
    await_line(&mut s, "hx-7 charting done: Map hx-m");
    charted(&mut s);
    s.cfg.away.store(true, Ordering::SeqCst);
    s.key(key(KeyCode::Enter));
    await_line(&mut s, "hx-m.1 research started");
    let pane = pane(&w, "hx-m.1");
    await_line(
        &mut s,
        "hx-m.1 parked: its session asked while you were Away",
    );
    (w, s, pane)
}

#[test]
fn under_away_the_wake_parks_it_closes_its_pane_and_starts_the_next() {
    let (w, mut s, first) = parked();

    await_line(&mut s, "hx-m.2 research started");
    assert!(!pane_alive(&w, &first));
    assert!(!s
        .questions
        .iter()
        .any(|q| q.ticket.as_deref() == Some("hx-m.1")));
    let comments = w.called("bd comments add hx-m.1");
    assert_eq!(comments.len(), 1, "{comments:?}");
    assert!(comments[0].contains("/continue @hx-m.1"), "{comments:?}");
    let r = saved(&w)
        .research
        .into_iter()
        .find(|r| r.waypoint == "hx-m.1")
        .unwrap();
    assert!(r.parked);
    assert_eq!(r.pane, "");
    assert!(!r.session.id.is_empty(), "its session saved");
    s.close();
}

#[test]
fn continue_at_a_parked_research_waypoint_resumes_it_by_id_and_puts_its_question() {
    let (w, mut s, _) = parked();
    let id = saved(&w).research[0].session.id.clone();
    let before = w.calls().len();

    s.command("/continue @hx-m.1");

    await_line(&mut s, "hx-m.1 research resumed: claude");
    let starts = w.since(before, "herdr agent start h-hx-m-1-research");
    assert!(starts[0].contains(&format!("--resume {id}")), "{starts:?}");
    assert!(!s.cfg.away.load(Ordering::SeqCst), "Away goes off");
    await_question(&mut s, "hx-m.1");
    assert_eq!(s.questions[0].ticket.as_deref(), Some("hx-m.1"));
    assert!(matches!(
        s.questions[0].about,
        About::Asked(Ask::Wake { .. })
    ));
    s.close();
}

/// Polls the Shell for a while, as its run loop does.
fn wait_a_while(s: &mut Screen) {
    for _ in 0..50 {
        s.poll();
        thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn after_no_to_next_waypoint_running_research_finishes_and_none_starts() {
    let issues = vec![
        BdTicket {
            labels: vec![MAP.to_string()],
            no_epic: true,
            ..BdTicket::new("hx-m")
        },
        waypoint("hx-m.1", GRILLING),
        waypoint("hx-m.2", RESEARCH),
        waypoint("hx-m.3", RESEARCH),
        waypoint("hx-m.4", GRILLING),
        waypoint("hx-m.e1", EPIC),
    ];
    let w = world(issues, session);
    max_research(&w, 1);
    let mut s = started(&w);
    await_line(&mut s, "hx-7 charting done: Map hx-m");
    charted(&mut s);
    s.key(key(KeyCode::Enter));
    await_line(&mut s, "hx-m.1 Waypoint started");
    await_line(&mut s, "hx-m.2 research started");
    await_line(&mut s, "hx-m.3 waits for a research slot, 1 of 1 running");
    w.run(&w.repo, &["bd", "close", "hx-m.1"]).unwrap();
    std::fs::write(saved(&w).result, "STATUS: done\nWAYPOINT: hx-m.1\n").unwrap();
    await_question(&mut s, "hx-m.1");

    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Enter));
    await_line(&mut s, "Brainstorm stopped");
    await_line(&mut s, "hx-m.2 research started");
    let second = pane(&w, "hx-m.2");
    finish(&w, "hx-m.2", true);

    await_line(&mut s, "hx-m.2 closed; its pane closes");
    assert!(!pane_alive(&w, &second));
    wait_a_while(&mut s);
    assert!(w.called("herdr agent start h-hx-m-3-research").is_empty());
    assert!(w.called("bd update hx-m.3").is_empty());
    s.close();
}

#[test]
fn with_research_with_you_none_starts() {
    let w = world(research(2), session);
    let mut s = started(&w);
    await_line(&mut s, "hx-7 charting done: Map hx-m");
    charted(&mut s);
    s.key(key(KeyCode::Char(' ')));
    s.key(key(KeyCode::Enter));

    await_line(&mut s, "hx-m live: research with you");
    await_line(&mut s, "hx-m.1 Waypoint started");
    wait_a_while(&mut s);
    assert!(w.called("herdr agent start h-hx-m-1-research").is_empty());
    assert!(w.called("herdr tab create").is_empty());
    assert!(s.research.is_empty());
    s.close();
}

#[test]
fn a_research_close_that_frees_a_waypoint_for_you_asks_next_waypoint() {
    let issues = vec![
        BdTicket {
            labels: vec![MAP.to_string()],
            no_epic: true,
            ..BdTicket::new("hx-m")
        },
        waypoint("hx-m.1", RESEARCH),
        BdTicket {
            deps: vec!["hx-m.1".to_string()],
            ..waypoint("hx-m.2", GRILLING)
        },
        waypoint("hx-m.e1", EPIC),
    ];
    let (w, mut s) = live(issues, session);
    await_line(&mut s, "hx-m waiting for research");
    await_line(&mut s, "hx-m.1 research started");

    finish(&w, "hx-m.1", true);

    await_line(&mut s, "hx-m.1 closed; its pane closes");
    await_question(&mut s, "hx-m.1");
    let q = &s.questions[0];
    assert_eq!(q.text, "1 Ticket hx-m.1 closed. Next Waypoint?");
    assert!(matches!(&q.about, About::Asked(Ask::NextWaypoint { map, .. }) if map == "hx-m"));
    assert_eq!(s.options()[0], "yes: the next on the Map, 2 Ticket hx-m.2");
    s.close();
}
