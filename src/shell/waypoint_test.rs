//! The Brainstorm loop over the fake world: a live Map's frontier starts
//! brainstorm-waypoint beside the Shell, its result asks "Next Waypoint?",
//! and /continue @<waypoint> starts one or says why not.

use crossterm::event::KeyCode;
use std::sync::Arc;

use super::chart_test::{await_prompt, map, pane_alive, saved, started, waypoint, world};
use super::shell_test::{await_line, await_until, key, line, notice, type_in};
use super::Screen;
use crate::brainstorm::{Phase, Research, EPIC, GRILLING, MAP, RESEARCH};
use crate::orchestrator::stage::Ask;
use crate::orchestrator::state::{Session, LOCAL};
use crate::orchestrator::world::{BdTicket, Prompt, World};
use crate::tools::Tools;

/// Charting writes Map hx-m; a Waypoint session idles.
fn session(p: &Prompt) -> (String, String) {
    match p.stage.as_str() {
        "chart" => ("STATUS: done\nMAP: hx-m\n".to_string(), "idle".to_string()),
        _ => (String::new(), "idle".to_string()),
    }
}

/// The prompt that named `result`, once the session has taken it: its
/// line is said before the prompt goes in.
fn prompt(w: &World, result: &std::path::Path) -> String {
    await_prompt(w, &result.display().to_string())
}

/// Idea hx-7 charted into Map hx-m over `issues`, and Start Map pressed
/// with research in the background.
fn live(issues: Vec<BdTicket>) -> (Arc<World>, Screen) {
    live_in(world(issues, session))
}

/// Idea hx-7 charted into Map hx-m in `w`, and Start Map pressed.
pub(super) fn live_in(w: Arc<World>) -> (Arc<World>, Screen) {
    let mut s = started(&w);
    await_line(&mut s, "hx-7 charting done: Map hx-m");
    s.key(key(KeyCode::Enter));
    (w, s)
}

#[test]
fn a_live_map_starts_brainstorm_waypoint_on_its_frontier() {
    let (w, mut s) = live(map(1));

    await_line(&mut s, "hx-m.1 Waypoint started: claude (pane 1-2)");

    let result = w.repo.join(LOCAL).join("brainstorms/hx-7/waypoint-1.md");
    let prompt = prompt(&w, &result);
    for want in [
        "# Brainstorm Waypoint",
        "- MAP: hx-m\n",
        "- BACKGROUND: on\n",
        "- PROMPT: none\n",
        &format!("- RESULT FILE: {}\n", result.display()),
    ] {
        assert!(prompt.contains(want), "{want:?} not in:\n{prompt}");
    }
    assert!(!prompt.contains("- WAYPOINT:"), "{prompt}");
    let start = w.called("herdr agent start").pop().unwrap();
    assert!(start.contains("h-hx-7-waypoint"), "{start}");
    s.close();
}

#[test]
fn research_alone_on_the_frontier_in_the_background_waits_with_no_session() {
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
    let (w, mut s) = live(issues);

    await_line(
        &mut s,
        "hx-m waiting for research: 1 running, 2 is blocked on 1",
    );

    let starts = w.called("herdr agent start");
    assert!(
        starts.iter().all(|c| !c.contains("-waypoint")),
        "{starts:?}"
    );
    assert_eq!(s.live.as_deref(), Some("hx-7"), "the Brainstorm stays live");
    s.close();
}

/// The live Map's first session started on hx-m.1, its pane.
fn working() -> (Arc<World>, Screen, String) {
    let (w, mut s) = live(map(1));
    await_line(&mut s, "hx-m.1 Waypoint started");
    let pane = saved(&w).pane;
    (w, s, pane)
}

/// The session's result naming `id`, written as it would.
fn result(w: &World, id: &str) {
    let file = saved(w).result;
    std::fs::write(file, format!("STATUS: done\nWAYPOINT: {id}\n")).unwrap();
}

/// `id` closed in bd, as its session closes it.
fn close(w: &World, id: &str) {
    w.run(&w.repo, &["bd", "close", id, "--reason", "decided"])
        .unwrap();
}

/// hx-m.1 closed by its session: the Question waits.
fn closed() -> (Arc<World>, Screen, String) {
    let (w, mut s, pane) = working();
    close(&w, "hx-m.1");
    result(&w, "hx-m.1");
    await_line(&mut s, "hx-m.1 closed; its pane closes");
    await_until(&mut s, "never asked", |s| !s.questions.is_empty());
    (w, s, pane)
}

#[test]
fn a_result_whose_waypoint_is_closed_closes_the_pane_and_asks_next_waypoint() {
    let (w, mut s, pane) = closed();

    assert!(!pane_alive(&w, &pane));
    let q = &s.questions[0];
    assert_eq!(q.ticket.as_deref(), Some("hx-m.1"));
    assert_eq!(q.text, "1 Ticket hx-m.1 closed. Next Waypoint?");
    assert!(matches!(
        &q.about,
        super::About::Asked(Ask::NextWaypoint { map, .. }) if map == "hx-m"
    ));
    assert!(q.brainstorms(), "no run's end drops it");
    assert_eq!(
        s.options(),
        [
            "yes: the next on the Map, 2 Ticket hx-m.2",
            "no: the Brainstorm stops, /continue @hx-m picks it up",
            "yes, with a prompt of your own in place of the default",
        ]
    );
    assert_eq!(saved(&w).pane, "");
    s.close();
}

#[test]
fn yes_starts_a_fresh_session_on_the_next_waypoint() {
    let (w, mut s, _) = closed();

    s.key(key(KeyCode::Enter));

    await_line(&mut s, "hx-m.2 Waypoint started");
    let result = w.repo.join(LOCAL).join("brainstorms/hx-7/waypoint-2.md");
    let prompt = prompt(&w, &result);
    assert!(prompt.contains("- PROMPT: none\n"), "{prompt}");
    assert!(s.questions.is_empty());
    s.close();
}

#[test]
fn yes_with_a_prompt_of_your_own_sends_it_as_prompt() {
    let (w, mut s, _) = closed();

    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Enter));
    type_in(&mut s, "start with the costs");
    s.key(key(KeyCode::Enter));

    await_line(&mut s, "hx-m.2 Waypoint started");
    let result = w.repo.join(LOCAL).join("brainstorms/hx-7/waypoint-2.md");
    let prompt = prompt(&w, &result);
    assert!(
        prompt.contains("- PROMPT: start with the costs\n"),
        "{prompt}"
    );
    s.close();
}

#[test]
fn no_stops_the_brainstorm_saved() {
    let (w, mut s, _) = closed();
    let starts = w.called("herdr agent start").len();

    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Enter));

    await_line(
        &mut s,
        "hx-m Brainstorm stopped: you answered no to next Waypoint?",
    );
    assert_eq!(s.live, None);
    assert_eq!(s.suggestion.as_deref(), Some("/continue @hx-m"));
    assert_eq!(w.called("herdr agent start").len(), starts);
    let b = saved(&w);
    assert_eq!(
        (b.phase, b.pane.as_str(), b.session),
        (Phase::Map, "", None)
    );
    s.close();
}

#[test]
fn a_result_whose_waypoint_is_still_open_keeps_the_pane() {
    let (w, mut s, pane) = working();

    result(&w, "hx-m.1");

    await_line(
        &mut s,
        "hx-m.1 result written, but 1 is still open in bd: pane 1-2 kept",
    );
    assert!(pane_alive(&w, &pane));
    assert!(s.questions.is_empty());
    s.close();
}

#[test]
fn an_idle_pane_with_no_result_asks_nothing() {
    let (w, mut s, pane) = working();

    for _ in 0..40 {
        s.poll();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    assert!(pane_alive(&w, &pane));
    assert!(s.questions.is_empty());
    assert!(line(s.events.last().unwrap()).contains("Waypoint started"));
    s.close();
}

/// hx-m.1 closed and no answered: the Map saved, nothing live.
fn stopped() -> (Arc<World>, Screen) {
    let (w, mut s, _) = closed();
    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Enter));
    await_line(&mut s, "Brainstorm stopped");
    (w, s)
}

#[test]
fn continue_a_waypoint_makes_its_map_live_and_starts_it_with_waypoint_set() {
    let (w, mut s) = stopped();

    s.command("/continue @hx-m.3");

    await_line(&mut s, "hx-m.3 Waypoint started");
    let result = w.repo.join(LOCAL).join("brainstorms/hx-7/waypoint-2.md");
    let prompt = prompt(&w, &result);
    assert!(prompt.contains("- WAYPOINT: hx-m.3\n"), "{prompt}");
    assert_eq!(s.live.as_deref(), Some("hx-7"));
    s.close();
}

#[test]
fn continue_a_waypoint_that_cannot_be_taken_says_why() {
    let (w, mut s) = stopped();
    w.lock()
        .tickets
        .iter_mut()
        .find(|t| t.id == "hx-m.4")
        .unwrap()
        .deps = vec!["hx-m.3".to_string()];
    s.brainstorms[0].research.push(Research {
        waypoint: "hx-m.5".to_string(),
        ..Default::default()
    });
    let starts = w.called("herdr agent start").len();

    for (id, why) in [
        ("hx-m.1", "refused: 1 is closed"),
        ("hx-m.4", "refused: 4 is blocked on 3"),
        (
            "hx-m.e1",
            "refused: e1 writes the Epic, and 4 other Waypoints are open",
        ),
        (
            "hx-m.5",
            "refused: 5 is a Research Waypoint a background session is running",
        ),
    ] {
        s.command(&format!("/continue @{id}"));
        assert_eq!(notice(&s), why);
    }
    assert_eq!(w.called("herdr agent start").len(), starts);
    assert_eq!(s.live, None);
    s.close();
}

#[test]
fn continue_a_waypoint_when_bd_list_fails_starts_nothing() {
    let (w, mut s) = stopped();
    let starts = w.called("herdr agent start").len();
    w.fail_once("bd list", "boom");

    s.command("/continue @hx-m.1");

    assert!(notice(&s).starts_with("bd list failed"), "{}", notice(&s));
    assert_eq!(w.called("herdr agent start").len(), starts);
    assert_eq!(s.live, None);
    s.close();
}

#[test]
fn an_interrupted_session_resumes_by_its_id() {
    let (w, mut s, pane) = working();
    s.close();
    w.run(&w.repo, &["herdr", "pane", "close", &pane]).unwrap();
    let mut b = saved(&w);
    b.session = Some(Session {
        app: "claude".to_string(),
        id: "s-old".to_string(),
        ..Default::default()
    });
    b.save(&w.repo).unwrap();
    let mut s = super::continue_test::reopened(&w);

    s.command("/continue @hx-m");
    s.key(key(KeyCode::Enter));

    await_line(&mut s, "hx-m Waypoint resumed: claude");
    let start = w.called("herdr agent start").pop().unwrap();
    assert!(
        start.contains("h-hx-7-waypoint") && start.contains(" --resume s-old"),
        "{start}"
    );
    let prompt = w.called("herdr agent prompt").pop().unwrap();
    assert!(prompt.ends_with(" continue"), "{prompt}");
    assert_eq!(saved(&w).result, b.result);
    s.close();
}

#[test]
fn manual_work_its_session_waits_on_is_asked_once_and_done_goes_into_its_pane() {
    let (w, mut s, pane) = working();
    let file = saved(&w).result;
    file_manual(&w, 1);
    let folder = w.repo.join(LOCAL).join("runs/hx-m.1/manual-work/1");

    await_line(&mut s, "hx-m.1 manual work in Waypoint (pane 1-2)");
    for _ in 0..20 {
        s.poll();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let asked = s
        .events
        .iter()
        .filter(|e| line(e).contains("asking you: manual work"));
    let asked: Vec<String> = asked.map(line).collect();
    assert_eq!(asked.len(), 1, "asked once: {asked:?}");
    assert!(matches!(
        s.questions[0].about,
        super::About::Asked(Ask::Manual { .. })
    ));
    assert!(s.questions[0].brainstorms(), "no run's end drops it");

    s.key(key(KeyCode::Enter)); // done

    let sent = w.called("herdr agent prompt").pop().unwrap();
    assert!(
        sent.starts_with(&format!("herdr agent prompt {pane} Manual work 1 done:")),
        "{sent}"
    );
    assert!(!std::path::Path::new(&file).exists(), "its result removed");
    assert!(!folder.exists(), "the item is marked done");
    assert_eq!(w.called("bd comments add hx-m.1").len(), 1);
    s.close();
}

#[test]
fn start_map_after_charting_with_session_ids_starts_fresh_not_charting_resumed() {
    let w = world(map(1), session);
    w.lock().integration = true;
    let (w, mut s) = live_in(w);

    await_line(&mut s, "hx-m.1 Waypoint started");

    let lines: Vec<String> = s.events.iter().map(line).collect();
    assert!(
        !lines.iter().any(|l| l.contains("not resumed")),
        "{lines:?}"
    );
    let start = w.called("herdr agent start").pop().unwrap();
    assert!(!start.contains("--resume"), "{start}");
    s.close();
}

#[test]
fn a_fresh_session_takes_the_frontier_over_a_claimed_waypoint() {
    let w = world(map(1), session);
    let claimed = |t: &mut BdTicket| t.status = "in_progress".to_string();
    w.lock()
        .tickets
        .iter_mut()
        .filter(|t| t.id == "hx-m.3")
        .for_each(claimed);
    let (w, mut s) = live_in(w);

    await_line(&mut s, "hx-m.1 Waypoint started");
    let result = w.repo.join(LOCAL).join("brainstorms/hx-7/waypoint-1.md");
    let prompt = prompt(&w, &result);
    assert!(!prompt.contains("- WAYPOINT:"), "{prompt}");
    s.close();
}

#[test]
fn an_interrupted_session_whose_resume_fails_takes_its_claimed_waypoint() {
    let (w, mut s, pane) = working();
    s.close();
    w.run(&w.repo, &["herdr", "pane", "close", &pane]).unwrap();
    w.lock()
        .tickets
        .iter_mut()
        .find(|t| t.id == "hx-m.1")
        .unwrap()
        .status = "in_progress".to_string();
    let mut b = saved(&w);
    b.session = Some(Session {
        app: "claude".to_string(),
        id: "s-unknown".to_string(),
        ..Default::default()
    });
    b.save(&w.repo).unwrap();
    w.fail_once("herdr agent start h-hx-7-waypoint", "no such session");
    let before = w.called("herdr agent prompt").len();
    let mut s = super::continue_test::reopened(&w);

    s.command("/continue @hx-m");
    s.key(key(KeyCode::Enter));

    await_line(&mut s, "hx-m Waypoint not resumed:");
    await_line(&mut s, "hx-m.1 Waypoint started");
    let fresh = || {
        let sent = w.called("herdr agent prompt").split_off(before);
        sent.into_iter()
            .find(|p| p.contains("# Brainstorm Waypoint"))
    };
    while fresh().is_none() {
        s.poll();
    }
    let prompt = fresh().unwrap();
    assert!(prompt.contains("- WAYPOINT: hx-m.1\n"), "{prompt}");
    s.close();
}

#[test]
fn continue_the_map_while_its_session_runs_is_refused() {
    let (w, mut s, _) = working();
    let starts = w.called("herdr agent start").len();

    s.command("/continue @hx-m");

    assert_eq!(notice(&s), "refused: a Waypoint session of hx-m is running");
    assert!(s.start_map.is_none());
    // a Continue form opened before the session started cannot start another
    let b = s.brainstorms[0].clone();
    s.open_start_map(&b, true);
    s.key(key(KeyCode::Enter));
    assert_eq!(notice(&s), "refused: a Waypoint session of hx-m is running");
    assert_eq!(w.called("herdr agent start").len(), starts);
    s.close();
}

#[test]
fn stopping_the_live_map_drops_its_next_waypoint_question() {
    let (_w, mut s, _) = closed();

    s.command("/brainstorm");
    s.command("y");

    assert!(!s.questions.iter().any(|q| q.brainstorms()));
    assert_eq!(s.live, None);
    s.close();
}

#[test]
fn next_waypoint_answered_once_its_map_is_not_live_starts_nothing() {
    let (w, mut s, _) = closed();
    let starts = w.called("herdr agent start").len();
    s.live = None;

    s.key(key(KeyCode::Enter));

    await_line(&mut s, "hx-m not acted on: the Map is no longer live");
    assert_eq!(w.called("herdr agent start").len(), starts);
    s.close();
}

#[test]
fn a_waypoint_pane_that_did_not_close_is_replaced_by_the_next_session() {
    let (w, mut s, pane) = working();
    w.fail_once("herdr pane close", "herdr is busy");
    close(&w, "hx-m.1");
    result(&w, "hx-m.1");
    await_line(&mut s, "hx-m.1 Waypoint pane not closed");
    await_until(&mut s, "never asked", |s| !s.questions.is_empty());
    assert_eq!(saved(&w).pane, pane);

    s.key(key(KeyCode::Enter)); // yes

    await_line(&mut s, "hx-m.2 Waypoint started");
    assert_ne!(saved(&w).pane, pane);
    s.close();
}

#[test]
fn parking_manual_work_stops_the_brainstorm_and_continue_asks_it_again() {
    let w = world(map(1), session);
    w.lock().integration = true;
    let (w, mut s) = live_in(w);
    await_line(&mut s, "hx-m.1 Waypoint started");
    file_manual(&w, 1);
    await_line(&mut s, "hx-m.1 manual work in Waypoint");

    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Enter)); // park

    await_line(
        &mut s,
        "hx-m.1 parked: Brainstorm stopped, /continue @hx-m asks again",
    );
    await_line(&mut s, "hx-m Waypoint stopped: its pane is gone");
    assert_eq!(s.live, None);
    assert_eq!(s.suggestion.as_deref(), Some("/continue @hx-m"));
    while s.driving("hx-7") {
        s.poll();
    }

    s.command("/continue @hx-m");
    s.key(key(KeyCode::Enter));

    await_line(&mut s, "hx-m Waypoint resumed");
    await_until(&mut s, "not asked again", |s| manual_asked(s) >= 2);
    s.close();
}

/// Manual work item `n` of hx-m.1, filed as its session files it, and the
/// result naming it written whole, as a rename writes it.
fn file_manual(w: &World, n: usize) {
    let folder = w
        .repo
        .join(LOCAL)
        .join(format!("runs/hx-m.1/manual-work/{n}"));
    std::fs::create_dir_all(&folder).unwrap();
    let item = "Ticket: hx-m.1 · Stage: waypoint · Blocks: yes\n\n## What\nSet the token.\n";
    std::fs::write(folder.join("manual-work.md"), item).unwrap();
    let file = saved(w).result;
    let tmp = format!("{file}.tmp");
    std::fs::write(&tmp, format!("STATUS: manual\n{}\n", folder.display())).unwrap();
    std::fs::rename(tmp, file).unwrap();
}

fn manual_asked(s: &Screen) -> usize {
    let manual =
        |e: &&crate::orchestrator::stage::Event| line(e).contains("manual work in Waypoint");
    s.events.iter().filter(manual).count()
}

#[test]
fn manual_work_filed_anew_before_a_tick_sees_no_result_is_asked_too() {
    let (w, mut s, _) = working();
    file_manual(&w, 1);
    await_line(&mut s, "hx-m.1 manual work in Waypoint");

    file_manual(&w, 2);

    await_until(&mut s, "item 2 never asked", |s| manual_asked(s) >= 2);
    s.close();
}

#[test]
fn manual_work_filed_again_in_the_folder_done_deleted_is_asked_too() {
    let (w, mut s, _) = working();
    file_manual(&w, 1);
    await_line(&mut s, "hx-m.1 asking you: manual work in Waypoint");
    let asked = manual_asked(&s);

    // Done deletes manual-work/1; the next item takes its number again
    // before a tick sees no result or no folder
    let folder = w.repo.join(LOCAL).join("runs/hx-m.1/manual-work/1");
    let next = folder.with_extension("new");
    std::fs::create_dir_all(&next).unwrap();
    std::fs::copy(folder.join("manual-work.md"), next.join("manual-work.md")).unwrap();
    std::fs::remove_dir_all(&folder).unwrap();
    std::fs::rename(&next, &folder).unwrap();
    file_manual(&w, 1);

    await_until(&mut s, "the new item 1 never asked", |s| {
        manual_asked(s) != asked
    });
    s.close();
}

#[test]
fn manual_work_whose_park_or_done_fails_is_asked_still() {
    let (w, mut s, _) = working();
    file_manual(&w, 1);
    await_line(&mut s, "hx-m.1 manual work in Waypoint");
    let file = saved(&w).result;
    let asked = |s: &Screen| {
        matches!(
            s.questions[0].about,
            super::About::Asked(Ask::Manual { .. })
        )
    };

    w.fail_once("herdr pane close", "herdr is busy");
    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Enter)); // park
    await_line(&mut s, "not stopped: its pane did not close");
    assert!(asked(&s), "the Question went with the park that failed");
    assert_eq!(s.live.as_deref(), Some("hx-7"));

    w.fail_once("herdr agent prompt", "herdr is busy");
    s.key(key(KeyCode::Up));
    s.key(key(KeyCode::Up));
    s.key(key(KeyCode::Enter)); // done
    await_line(&mut s, "hx-m.1 never took your answer");
    assert!(asked(&s), "the Question went with the answer never sent");
    assert!(std::path::Path::new(&file).exists(), "its result put back");
    s.close();
}

#[test]
fn a_run_ending_keeps_the_prompt_composed_for_next_waypoint() {
    let mut issues = map(1);
    issues.push(BdTicket::new("hx-1"));
    let (w, mut s) = live(issues);
    await_line(&mut s, "hx-m.1 Waypoint started");
    close(&w, "hx-m.1");
    result(&w, "hx-m.1");
    await_until(&mut s, "never asked", |s| !s.questions.is_empty());
    s.command("/start-ticket hx-1");
    await_line(&mut s, "hx-1 implement started");
    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Enter)); // a prompt of your own
    assert!(s.composing);

    s.command("/stop-work");
    super::shell_test::await_end(&mut s);

    assert!(
        s.composing,
        "the prompt was for Next Waypoint?, which stays"
    );
    assert!(matches!(
        s.questions[0].about,
        super::About::Asked(Ask::NextWaypoint { .. })
    ));
    s.close();
}
