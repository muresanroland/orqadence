//! /continue @ over the fake world: a stopped charting resumes, a Map's
//! Continue form opens, the @ list shows the Brainstorm's issues, and one
//! Brainstorm is live at a time.

use crossterm::event::KeyCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use super::chart_test::{idle, map, pane_alive, saved, started, world};
use super::shell_test::{await_line, key, notice, shell, type_in, type_line};
use super::Screen;
use crate::brainstorm::{self, Brainstorm, Phase, Research, RESEARCH};
use crate::orchestrator::state::{Session, TicketState, STATUS_PARKED};
use crate::orchestrator::world::{BdTicket, World};
use crate::tools::Tools;

/// The Shell opened again over `w`, its Brainstorms loaded as at open.
fn reopened(w: &Arc<World>) -> Screen {
    let mut s = shell(w);
    s.shell_pane = "w1:shell".to_string();
    let issues = s.reload_issues().unwrap_or_default();
    s.brainstorms = brainstorm::load(&w.repo, &issues);
    s
}

/// Charting hx-7 started, then the Shell closed: its pane left running.
fn stopped_shell(w: &Arc<World>) {
    let mut s = started(w);
    s.close();
}

#[test]
fn continue_an_idea_whose_pane_is_alive_watches_it_again() {
    let w = world(Vec::new(), idle);
    stopped_shell(&w);
    let pane = saved(&w).pane;
    let (splits, starts) = (
        w.called("herdr pane split").len(),
        w.called("herdr agent start").len(),
    );
    let mut s = reopened(&w);

    s.command("/continue @hx-7");
    await_line(&mut s, "hx-7 charting watched again (pane");

    assert_eq!(w.called("herdr pane split").len(), splits);
    assert_eq!(w.called("herdr agent start").len(), starts);
    assert_eq!(saved(&w).pane, pane);
    assert_eq!(s.live.as_deref(), Some("hx-7"));
    s.close();
}

#[test]
fn continue_an_idea_whose_session_is_gone_resumes_it_by_id() {
    let w = world(Vec::new(), idle);
    stopped_shell(&w);
    let mut b = saved(&w);
    w.lock().names.clear(); // its pane open, no agent in it
    b.session = Some(Session {
        app: "claude".to_string(),
        id: "s-old".to_string(),
        ..Default::default()
    });
    b.save(&w.repo).unwrap();
    let mut s = reopened(&w);

    s.command("/continue @hx-7");
    await_line(&mut s, "hx-7 charting resumed: claude");

    let start = w.called("herdr agent start").pop().unwrap();
    assert!(start.contains(" --resume s-old"), "{start}");
    let prompt = w.called("herdr agent prompt").pop().unwrap();
    assert!(prompt.ends_with(" continue"), "{prompt}");
    assert_eq!(s.live.as_deref(), Some("hx-7"));
    s.close();
}

#[test]
fn continue_an_idea_whose_resume_fails_starts_brainstorm_chart_fresh() {
    let w = world(Vec::new(), idle);
    stopped_shell(&w);
    let mut b = saved(&w);
    w.run(&w.repo, &["herdr", "pane", "close", &b.pane])
        .unwrap();
    b.session = Some(Session {
        app: "claude".to_string(),
        id: "s-unknown".to_string(),
        ..Default::default()
    });
    b.save(&w.repo).unwrap();
    w.fail_once("herdr agent start h-hx-7-chart", "no such session");
    let before = w.called("herdr agent start").len();
    let mut s = reopened(&w);

    s.command("/continue @hx-7");
    await_line(&mut s, "hx-7 charting not resumed:");
    await_line(&mut s, "hx-7 charting started: claude");

    let starts = w.called("herdr agent start")[before..].to_vec();
    assert_eq!(starts.len(), 2, "{starts:?}");
    assert!(!starts[1].contains("--resume"), "{starts:?}");
    let prompt = w.called("herdr agent prompt").pop().unwrap();
    assert!(prompt.contains("- IDEA: hx-7"), "{prompt}");
    s.close();
}

/// Map hx-m's Brainstorm, from Idea hx-8, saved with research with you.
fn saved_map(w: &World) {
    let b = Brainstorm {
        phase: Phase::Map,
        idea: "hx-8".to_string(),
        map: "hx-m".to_string(),
        started: true,
        ..Default::default()
    };
    b.save(&w.repo).unwrap();
}

#[test]
fn continue_a_map_opens_its_continue_form_at_the_saved_answer() {
    let w = world(map(1), idle);
    saved_map(&w);
    let mut s = reopened(&w);

    s.command("/continue @hx-m");

    let m = s.start_map.as_ref().expect("no Continue form");
    assert_eq!((m.idea.as_str(), m.map.as_str()), ("hx-8", "hx-m"));
    assert!(m.again && !m.background);
    s.key(key(KeyCode::Enter));
    assert_eq!(s.live.as_deref(), Some("hx-8"));
    s.close();
}

#[test]
fn continue_another_while_one_is_live_asks_and_yes_stops_the_live_one_saved() {
    let w = world(map(1), idle);
    saved_map(&w);
    let mut s = started(&w);
    s.brainstorms = reopened(&w).brainstorms;
    let pane = saved(&w).pane;

    s.command("/continue @hx-m");

    assert!(s.start_map.is_none());
    assert_eq!(
        s.questions[0].text,
        "/continue @hx-m: hx-7 is live. Stop hx-7 and start this one?"
    );
    s.command("y");
    assert!(!pane_alive(&w, &pane));
    await_line(
        &mut s,
        "hx-7 charting stopped: its pane is gone; Brainstorm saved",
    );
    assert!(saved(&w).pane.is_empty());
    assert!(s.start_map.as_ref().is_some_and(|m| m.map == "hx-m"));
    s.close();
}

#[test]
fn brainstorm_while_one_is_live_asks_and_no_keeps_it() {
    let w = world(Vec::new(), idle);
    let mut s = started(&w);
    let pane = saved(&w).pane;

    type_line(&mut s, "/brainstorm");

    assert!(s.idea.is_none());
    assert_eq!(
        s.questions[0].text,
        "/brainstorm: hx-7 is live. Stop hx-7 and start this one?"
    );
    s.command("n");
    assert!(pane_alive(&w, &pane));
    assert_eq!(s.live.as_deref(), Some("hx-7"));
    s.close();
}

#[test]
fn stop_work_leaves_the_brainstorm_live() {
    let w = world(Vec::new(), idle);
    let mut s = started(&w);
    let pane = saved(&w).pane;

    type_line(&mut s, "/stop-work");

    assert_eq!(s.live.as_deref(), Some("hx-7"));
    assert!(pane_alive(&w, &pane));
    assert!(w.called("herdr pane close").is_empty());
    s.close();
}

#[test]
fn closing_the_charting_pane_ends_the_live_brainstorm() {
    let w = world(Vec::new(), idle);
    let mut s = started(&w);
    let pane = saved(&w).pane;

    w.run(&w.repo, &["herdr", "pane", "close", &pane]).unwrap();
    await_line(&mut s, "hx-7 charting stopped");
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.live.is_some() {
        assert!(Instant::now() < deadline, "still live");
        s.poll();
        thread::sleep(Duration::from_millis(1));
    }
    s.close();
}

#[test]
fn the_continue_list_shows_maps_waypoints_ideas_and_parked_tickets_with_their_state() {
    let mut issues = map(1);
    issues[2].deps = vec!["hx-m.3".to_string()];
    issues.push(BdTicket {
        no_epic: true,
        ..BdTicket::new("hx-p")
    });
    let w = world(issues, idle);
    w.lock().tickets[2].status = "closed".to_string(); // hx-m.1, after the Idea and the Map
    stopped_shell(&w);
    saved_map(&w);
    let mut s = reopened(&w);
    s.state.tickets.insert(
        "hx-p".to_string(),
        TicketState {
            status: STATUS_PARKED.to_string(),
            ..Default::default()
        },
    );

    type_in(&mut s, "/continue @");

    let rows: Vec<String> = s
        .list()
        .iter()
        .map(|(id, mid, text)| format!("{id} | {mid} | {text}"))
        .collect();
    assert_eq!(
        rows,
        [
            "hx-m | Map | Ticket hx-m · 5 Waypoints open, stopped",
            "hx-m.2 | Waypoint | Ticket hx-m.2 · blocked on 3",
            "hx-m.3 | Waypoint | Ticket hx-m.3",
            "hx-m.4 | Waypoint | Ticket hx-m.4",
            "hx-m.5 | Waypoint | Ticket hx-m.5",
            "hx-m.e1 | Waypoint | Ticket hx-m.e1 · last, 4 others open",
            "hx-m.1 | Waypoint | Ticket hx-m.1 · closed",
            "hx-7 | Idea | Ticket hx-7 · charting stopped",
            "hx-p | Ticket | Ticket hx-p · parked",
        ]
    );
    s.close();
}

#[test]
fn the_continue_list_says_live_not_started_and_research() {
    let mut issues = map(1);
    issues[1].labels = vec![RESEARCH.to_string()];
    issues[2].labels = vec![RESEARCH.to_string()];
    let w = world(issues, idle);
    let mut b = Brainstorm {
        phase: Phase::Map,
        idea: "hx-8".to_string(),
        map: "hx-m".to_string(),
        ..Default::default()
    };
    for (waypoint, parked) in [("hx-m.1", false), ("hx-m.2", true)] {
        b.research.push(Research {
            waypoint: waypoint.to_string(),
            parked,
            ..Default::default()
        });
    }
    b.save(&w.repo).unwrap();
    let mut s = reopened(&w);

    type_in(&mut s, "/continue @hx-m");
    let text = |s: &Screen, id: &str| {
        let rows = s.list();
        rows.iter()
            .find(|r| r.0 == id)
            .map(|r| r.2.clone())
            .unwrap()
    };
    assert_eq!(text(&s, "hx-m"), "Ticket hx-m · not started");
    assert_eq!(text(&s, "hx-m.1"), "Ticket hx-m.1 · researching");
    assert_eq!(text(&s, "hx-m.2"), "Ticket hx-m.2 · parked, asks you");

    s.live = Some("hx-8".to_string());
    assert_eq!(text(&s, "hx-m"), "Ticket hx-m · 6 Waypoints open, live");
    s.close();
}

#[test]
fn continue_the_idea_already_charting_is_refused() {
    let w = world(Vec::new(), idle);
    let mut s = started(&w);
    let starts = w.called("herdr agent start").len();

    s.command("/continue @hx-7");

    assert_eq!(notice(&s), "refused: hx-7 is already charting");
    assert_eq!(w.called("herdr agent start").len(), starts);
    s.close();
}

#[test]
fn the_charting_session_dying_in_its_open_pane_stops_the_brainstorm_saved() {
    let w = world(Vec::new(), idle);
    let mut s = started(&w);
    let pane = saved(&w).pane;

    w.lock().agents.remove(&pane);
    await_line(
        &mut s,
        "hx-7 charting stopped: its session is gone; Brainstorm saved",
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.live.is_some() {
        assert!(Instant::now() < deadline, "still live");
        s.poll();
        thread::sleep(Duration::from_millis(1));
    }

    assert!(pane_alive(&w, &pane));
    assert_eq!(saved(&w).pane, pane, "a resume replaces the pane");
    s.close();
}

#[test]
fn continue_an_idea_when_herdr_does_not_answer_is_refused() {
    let w = world(Vec::new(), idle);
    stopped_shell(&w);
    let pane = saved(&w).pane;
    let (splits, starts) = (
        w.called("herdr pane split").len(),
        w.called("herdr agent start").len(),
    );
    let mut s = reopened(&w);
    w.fail_once("herdr agent get", "herdr: timeout");

    s.command("/continue @hx-7");
    await_line(
        &mut s,
        "hx-7 charting not resumed: herdr did not answer, try again",
    );

    assert!(pane_alive(&w, &pane));
    assert_eq!(saved(&w).pane, pane);
    assert_eq!(w.called("herdr pane split").len(), splits);
    assert_eq!(w.called("herdr agent start").len(), starts);
    s.close();
}

#[test]
fn switching_while_the_charting_pane_is_still_opening_is_refused() {
    let w = world(map(1), idle);
    saved_map(&w);
    // the driver held before its pane: the Shell's pane read blocks
    let opening = Arc::new(AtomicBool::new(true));
    let held = opening.clone();
    w.hook(move |_, argv| {
        if argv.starts_with(&["bd", "create"]) {
            return Some(Ok("hx-7\n".to_string()));
        }
        while argv == ["herdr", "pane", "get", "w1:shell"] && held.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(1));
        }
        None
    });
    let mut s = reopened(&w);
    type_line(&mut s, "/brainstorm");
    type_in(&mut s, "Queue bd writes");
    s.key(key(KeyCode::Enter));
    assert_eq!(s.live.as_deref(), Some("hx-7"));

    s.command("/continue @hx-m");
    s.command("y");

    assert_eq!(
        notice(&s),
        "refused: hx-7's pane is still opening, try again"
    );
    assert!(s.start_map.is_none());
    assert_eq!(s.live.as_deref(), Some("hx-7"));
    opening.store(false, Ordering::SeqCst);
    await_line(&mut s, "hx-7 charting started");
    s.close();
}

#[test]
fn switching_while_a_resume_replaces_the_dead_sessions_pane_is_refused() {
    let w = world(map(1), idle);
    stopped_shell(&w);
    let mut b = saved(&w);
    let pane = b.pane.clone();
    w.lock().names.clear(); // its pane open, no agent in it
    b.session = Some(Session {
        app: "claude".to_string(),
        id: "s-old".to_string(),
        ..Default::default()
    });
    b.save(&w.repo).unwrap();
    saved_map(&w);
    // the driver held before its new pane: the Shell's pane read blocks
    let (opening, reached) = (
        Arc::new(AtomicBool::new(true)),
        Arc::new(AtomicBool::new(false)),
    );
    let (held, at) = (opening.clone(), reached.clone());
    w.hook(move |_, argv| {
        while argv == ["herdr", "pane", "get", "w1:shell"] && held.load(Ordering::SeqCst) {
            at.store(true, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(1));
        }
        None
    });
    let mut s = reopened(&w);
    s.command("/continue @hx-7");
    while !reached.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(1));
    }

    s.command("/continue @hx-m");
    s.command("y");

    assert_eq!(
        notice(&s),
        "refused: hx-7's pane is still opening, try again"
    );
    assert!(pane_alive(&w, &pane));
    assert!(s.start_map.is_none());
    assert_eq!(s.live.as_deref(), Some("hx-7"));
    opening.store(false, Ordering::SeqCst);
    await_line(&mut s, "hx-7 charting resumed: claude");
    assert_ne!(saved(&w).pane, pane);
    s.close();
}
