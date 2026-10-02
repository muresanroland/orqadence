//! The Release at a run's end: orqa:release read on the Epic, or on any
//! Ticket of a Ticket run, and the Release Stage the run owns.

use super::app::{set_switch, RELEASE_ON};
use super::judgment::Action;
use super::limit_test::{at, hits, now, CLAUDE};
use super::pr_test::agent_merge;
use super::question_test::ASKS;
use super::release::RELEASE_LABEL;
use super::scheduler_test::run_epic;
use super::stage::{Answer, Ask, Orchestrator};
use super::state::{
    Release, Session, TicketState, STATUS_MERGED, STATUS_PARKED, STATUS_PR_OPEN, STATUS_RUNNING,
};
use super::world::{
    new_world, restarted, set_clock, spawn_epic, succeed, wait_until, working, BdTicket, Prompt,
    Running, World,
};
use super::write_file;
use chrono::{DateTime, Local, TimeDelta};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

/// The version PR the Release opens.
pub(crate) const VERSION_PR: &str = "https://example.test/pr/version";

/// The Release's result: the new version and the version PR.
const RELEASED: &str = "STATUS: done\nVERSION: v1.5.0\nPR: https://example.test/pr/version\n";

/// The Release label, as bd lists it on an issue.
pub(crate) fn label() -> Vec<String> {
    vec![RELEASE_LABEL.to_string()]
}

/// Every Stage succeeds; the Release writes RELEASED.
fn release_done(p: &Prompt) -> (String, String) {
    match p.stage == "release" {
        true => (RELEASED.to_string(), "idle".to_string()),
        false => succeed(p),
    }
}

/// A world with releases on, every PR merged as soon as gh is asked.
pub(crate) fn releasing(tickets: Vec<BdTicket>) -> (Arc<World>, Orchestrator) {
    let (w, o) = new_world(tickets);
    set_switch(&w.repo, &RELEASE_ON, true).unwrap();
    w.lock().merged = true;
    w.session(release_done);
    (w, o)
}

/// Whether the log has `text` as a run-level line: no id before it.
fn run_level(w: &World, text: &str) -> bool {
    w.log().lines().any(|l| {
        l.splitn(3, ' ')
            .nth(2)
            .is_some_and(|rest| rest.starts_with(text))
    })
}

/// Answers the tag Question `answer` once it is put, then waits for the
/// run to end.
fn tag(w: &World, o: &Orchestrator, run: &mut Running, answer: &str) {
    let asked = w.await_event(" and push it?");
    let id = asked.ticket.unwrap();
    o.answer(&id, "", Answer::Prompt(answer.to_string()));
    run.wait();
}

/// An Epic run to its end, the tag Question answered no.
fn run_tagged(w: &World, o: &Arc<Orchestrator>) {
    let mut run = spawn_epic(o.clone(), "hx");
    tag(w, o, &mut run, "no");
    o.wait_in_flight();
}

#[test]
fn the_version_prs_merge_puts_the_tag_question_and_yes_tags_the_merge_commit_and_pushes_it() {
    let (w, o) = releasing(vec![BdTicket::new("hx-1")]);
    w.lock().epic_labels = label();
    let o = Arc::new(o);

    let mut run = spawn_epic(o.clone(), "hx");

    let asked = w.await_event("Tag v1.5.0 and push it?");
    let Some(Ask::Tag { options, .. }) = &asked.ask else {
        panic!("no tag Question: {asked:?}");
    };
    assert_eq!(options, &["yes", "no"]);
    assert_eq!(asked.ticket.as_deref(), Some("release-hx"));
    w.await_line("version PR #version merged");
    let worktree = o.worktree("release-hx").display().to_string();
    let removed = format!("bd worktree remove {worktree} --force");
    assert_eq!(w.called(&removed).len(), 1, "the Release's worktree stayed");
    assert_eq!(w.called("git branch -D release-hx").len(), 1);
    assert!(
        !run.finished_within(Duration::from_millis(30)),
        "the run ended before the tag Question was answered"
    );

    let before = w.calls().len();
    tag(&w, &o, &mut run, "yes");

    assert_eq!(
        w.since(before, "git "),
        [
            "git fetch origin HEAD",
            "git tag v1.5.0 m3rg3d",
            "git push origin v1.5.0"
        ]
    );
    assert!(w.called("gh release").is_empty(), "a GitHub Release made");
    assert!(run_level(&w, "tagged v1.5.0 and pushed"));
    let release = o.state.lock().unwrap().release.clone().unwrap();
    assert!(release.tagged, "{release:?}");
    assert_eq!(release.commit, "m3rg3d");
}

#[test]
fn no_to_the_tag_question_tags_nothing_says_how_and_ends_the_run() {
    let (w, o) = releasing(vec![BdTicket::new("hx-1")]);
    w.lock().epic_labels = label();
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");

    w.await_event("Tag v1.5.0 and push it?");
    tag(&w, &o, &mut run, "no");

    assert!(w.called("git tag").is_empty() && w.called("git push").is_empty());
    let how = "git fetch origin HEAD && git tag v1.5.0 m3rg3d && git push origin v1.5.0";
    assert!(run_level(&w, &format!("v1.5.0 not tagged: {how}")));
    assert!(!o.state.lock().unwrap().release.clone().unwrap().tagged);
}

#[test]
fn a_repo_that_keeps_its_version_in_tags_alone_gets_the_tag_question_at_once() {
    let (w, o) = releasing(vec![BdTicket::new("hx-1")]);
    w.lock().epic_labels = label();
    w.session(|p| match p.stage == "release" {
        true => (
            "STATUS: done\nVERSION: v1.5.0\n".to_string(),
            "idle".to_string(),
        ),
        false => succeed(p),
    });
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");

    w.await_event("Tag v1.5.0 and push it?");
    let calls = w.calls();
    let prompted = calls
        .iter()
        .position(|c| c.starts_with("herdr agent prompt") && c.contains("release.md"))
        .unwrap();
    let polled: Vec<&String> = calls[prompted..]
        .iter()
        .filter(|c| c.starts_with("gh "))
        .collect();
    assert!(polled.is_empty(), "polled with no version PR: {polled:?}");

    tag(&w, &o, &mut run, "yes");
    assert_eq!(
        w.called("git tag"),
        ["git tag v1.5.0 FETCH_HEAD"],
        "origin's default branch head as fetched"
    );
}

#[test]
fn a_version_pr_closed_unmerged_asks_and_run_again_starts_a_fresh_release() {
    let (w, o) = releasing(vec![BdTicket::new("hx-1")]);
    {
        let mut world = w.lock();
        world.epic_labels = label();
        world.integration = true; // a session id, which a resume would use
        let closed = r#"{"state":"CLOSED","mergeable":"UNKNOWN"}"#.to_string();
        world.prs.insert(VERSION_PR.to_string(), closed);
    }
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");

    let asked = w.await_event("version PR #version closed without merging: run the Release again?");
    let Some(Ask::ReleaseAgain { options }) = &asked.ask else {
        panic!("no Question: {asked:?}");
    };
    assert_eq!(options, &["run the Release again", "end without a Release"]);
    assert!(run_level(&w, "version PR #version closed without merging"));
    let worktree = o.worktree("release-hx").display().to_string();
    let removed = format!("bd worktree remove {worktree} --force");
    assert!(w.called(&removed).is_empty(), "removed before the answer");

    o.answer("release-hx", "", Answer::Prompt(options[0].clone()));

    w.await_nth("run the Release again?", 2);
    assert_eq!(w.called(&removed).len(), 1);
    assert_eq!(w.called("git branch -D release-hx").len(), 1);
    let starts = w.called("herdr agent start h-release-hx-release ");
    assert_eq!(starts.len(), 2, "{starts:?}");
    assert!(
        !starts[1].contains("--resume"),
        "not a fresh session: {starts:?}"
    );
    let create = format!("bd worktree create {worktree} --branch release-hx");
    assert_eq!(w.called(&create).len(), 2, "a fresh worktree");

    o.answer(
        "release-hx",
        "",
        Answer::Prompt("end without a Release".to_string()),
    );
    run.wait();
    assert!(!o.stopping(), "the run stopped rather than ended");
    assert!(w.called("git tag").is_empty());
}

#[test]
fn an_epic_carrying_orqa_release_ends_in_a_release_with_bump_minor_and_each_prs_url() {
    let (w, o) = releasing(vec![BdTicket::new("hx-1"), BdTicket::new("hx-2")]);
    w.lock().epic_labels = label();
    let o = Arc::new(o);

    run_tagged(&w, &o);

    let prompt = w.prompt("release-hx/release.md");
    for want in [
        "- Bump: minor\n",
        "- Epic: hx Epic hx\n",
        "\n  - hx-1 Ticket hx-1: https://example.test/pr/hx-1",
        "\n  - hx-2 Ticket hx-2: https://example.test/pr/hx-2",
    ] {
        assert!(
            prompt.contains(want),
            "the Release's prompt lacks {want:?}:\n{prompt}"
        );
    }
    let worktree = o.worktree("release-hx").display().to_string();
    assert_eq!(
        w.called(&format!(
            "bd worktree create {worktree} --branch release-hx"
        ))
        .len(),
        1,
        "the Release's worktree"
    );
    assert!(
        w.called("bd update release-hx").is_empty(),
        "bd was asked to update the Release: it has no bd issue"
    );
    assert_eq!(
        w.called("herdr tab create --workspace w1 --label release-hx ")
            .len(),
        1,
        "the Release's tab of its own"
    );
    let lines = w.lines();
    let at = |text: &str| {
        lines
            .iter()
            .position(|l| l.starts_with(text))
            .unwrap_or_else(|| panic!("no line {text:?}:\n{}", lines.join("\n")))
    };
    assert!(at("Epic done, every Ticket closed") < at("release started: claude"));
    assert!(at("release started: claude") < at("release done: v1.5.0"));
    assert!(at("release done: v1.5.0") < at("version PR #version opened"));
    assert!(run_level(
        &w,
        "version PR #version opened (https://example.test/pr/version)"
    ));
    let release = o.state.lock().unwrap().release.clone().unwrap();
    assert_eq!(
        (
            release.id.as_str(),
            release.version.as_str(),
            release.ts.pr.as_str()
        ),
        ("release-hx", "v1.5.0", "https://example.test/pr/version")
    );
}

#[test]
fn an_earlier_releases_worktree_gives_a_new_epic_release_an_id_of_its_own() {
    let (w, mut o) = releasing(vec![BdTicket::new("hx-1")]);
    w.lock().epic_labels = label();
    set_clock(&mut o.cfg, now());
    std::fs::create_dir_all(o.worktree("release-hx")).unwrap(); // its record discarded
    let o = Arc::new(o);

    run_tagged(&w, &o);

    w.await_line("release done: v1.5.0");
    let id = o.state.lock().unwrap().release.clone().unwrap().id;
    assert_eq!(id, "release-hx-2026-09-25-140000");
    let worktree = o.worktree(&id).display().to_string();
    let create = format!("bd worktree create {worktree} --branch {id}");
    assert_eq!(w.called(&create).len(), 1, "a worktree off today's main");
}

/// A Ticket run, as /start-ticket begins one, over `tickets`, to its end.
fn run_tickets(w: &World, o: &Arc<Orchestrator>, tickets: &[&str]) {
    let ids: Vec<String> = tickets.iter().map(|t| t.to_string()).collect();
    assert!(o.enqueue(&ids));
    let mut run = spawn_epic(o.clone(), "");
    tag(w, o, &mut run, "no");
    o.wait_in_flight();
    w.await_line("Ticket run done, every Ticket closed");
}

#[test]
fn a_ticket_run_whose_queued_ticket_carries_orqa_release_gets_bump_patch() {
    let (w, mut o) = releasing(vec![
        BdTicket::new("hx-1"),
        BdTicket {
            labels: label(),
            ..BdTicket::new("hx-2")
        },
    ]);
    set_clock(&mut o.cfg, now());
    let o = Arc::new(o);

    run_tickets(&w, &o, &["hx-1", "hx-2"]);

    let id = o.state.lock().unwrap().release.clone().unwrap().id;
    assert_eq!(id, "release-2026-09-25-140000", "its date and time");
    let prompt = w.prompt(&format!("{id}/release.md"));
    for want in [
        "- Bump: patch\n",
        "- Epic: none\n",
        "\n  - hx-1 Ticket hx-1: https://example.test/pr/hx-1",
        "\n  - hx-2 Ticket hx-2: https://example.test/pr/hx-2",
    ] {
        assert!(
            prompt.contains(want),
            "the Release's prompt lacks {want:?}:\n{prompt}"
        );
    }
    w.await_line("release done: v1.5.0");
}

/// The run ended as before the Release: no record, no session, no line.
fn no_release(w: &World, o: &Orchestrator) {
    assert_eq!(o.state.lock().unwrap().release, None);
    assert!(w.called("herdr agent start h-release").is_empty());
    let lines = w.lines();
    assert!(
        !lines.iter().any(|l| l.contains("release")),
        "a Release line: {lines:?}"
    );
}

#[test]
fn with_the_switch_off_or_the_label_on_the_epics_tickets_alone_the_run_ends_as_today() {
    // (the switch, the Epic's labels, its Ticket's)
    for (on, epic, ticket) in [(false, label(), vec![]), (true, vec![], label())] {
        let (w, o) = releasing(vec![BdTicket {
            labels: ticket,
            ..BdTicket::new("hx-1")
        }]);
        set_switch(&w.repo, &RELEASE_ON, on).unwrap();
        w.lock().epic_labels = epic;
        let o = Arc::new(o);

        run_epic(&o);

        w.await_line("Epic done, every Ticket closed");
        no_release(&w, &o);
    }
}

#[test]
fn a_run_where_no_ticket_merged_a_pr_gets_no_release() {
    let (w, o) = releasing(vec![BdTicket::new("hx-1")]);
    w.lock().epic_labels = label();
    w.lock().tickets[0].status = "closed".to_string(); // closed by hand
    let o = Arc::new(o);

    run_epic(&o);

    w.await_line("Epic done, every Ticket closed");
    no_release(&w, &o);
}

#[test]
fn orqa_release_on_a_ticket_raises_no_label_question() {
    let (w, o) = new_world(vec![BdTicket {
        labels: label(),
        ..BdTicket::new("hx-1")
    }]);
    w.lock().merged = true;
    let o = Arc::new(o);

    run_epic(&o);

    w.await_line("hx-1 merged, Ticket closed");
    let asked: Vec<String> = w
        .events()
        .into_iter()
        .filter(|e| matches!(e.ask, Some(Ask::Labels { .. })))
        .map(|e| e.text)
        .collect();
    assert!(asked.is_empty(), "label Questions: {asked:?}");
    assert!(w.called("bd label remove").is_empty());
}

#[test]
fn a_parked_ticket_keeps_the_release_from_starting() {
    let (w, o) = releasing(vec![BdTicket::new("hx-1"), BdTicket::new("hx-2")]);
    w.lock().epic_labels = label();
    w.lock().tickets[1].status = "in_progress".to_string(); // started, then Parked
    o.update("hx-2", |ts| {
        ts.status = STATUS_PARKED.to_string();
        ts.reason = "by you at implement".to_string();
    });
    let o = Arc::new(o);

    let run = spawn_epic(o.clone(), "hx");

    w.await_line("hx-1 merged, Ticket closed");
    assert!(
        !run.finished_within(Duration::from_millis(50)),
        "the run ended with a Ticket Parked"
    );
    no_release(&w, &o);
}

#[test]
fn enqueue_is_refused_once_the_release_starts() {
    let (w, o) = releasing(vec![BdTicket {
        labels: label(),
        ..BdTicket::new("hx-1")
    }]);
    w.session(|p| match p.stage == "release" {
        true => working(p),
        false => succeed(p),
    });
    let o = Arc::new(o);
    assert!(o.enqueue(&["hx-1".to_string()]));

    let _run = spawn_epic(o.clone(), "");

    w.await_line("release started: claude");
    assert!(
        !o.enqueue(&["hx-2".to_string()]),
        "a Ticket joined the run once its Release started"
    );
    assert_eq!(o.state.lock().unwrap().queue, ["hx-1"]);
}

#[test]
fn the_release_is_never_resumable_nor_polled_for_a_merge() {
    let (w, o) = new_world(vec![]);
    o.state.lock().unwrap().release = Some(Box::new(Release {
        id: "release-hx".to_string(),
        ts: TicketState {
            status: STATUS_RUNNING.to_string(),
            stage: "release".to_string(),
            ..Default::default()
        },
        ..Default::default()
    }));
    assert!(o.resumable().is_empty(), "the Release resumed as a Ticket");

    o.update("release-hx", |ts| {
        ts.status = STATUS_PR_OPEN.to_string();
        ts.pr = "https://example.test/pr/version".to_string();
    });
    assert!(o.poll_merges().is_empty());
    assert!(
        w.called("gh ").is_empty(),
        "gh was asked about the version PR"
    );
    assert!(
        o.state.lock().unwrap().tickets.is_empty(),
        "the Release was recorded as a Ticket"
    );
}

/// A run stopped in its Release, every Ticket merged: the Release's session
/// id saved, its pane gone.
fn stopped_in_release(w: &World, o: &Orchestrator) {
    {
        let mut world = w.lock();
        world.integration = true;
        world.tickets[0].status = "closed".to_string();
        world.tickets[0].close_reason = "PR merged: https://example.test/pr/hx-1".to_string();
    }
    let mut state = o.state.lock().unwrap();
    state.tickets.insert(
        "hx-1".to_string(),
        TicketState {
            status: STATUS_MERGED.to_string(),
            pr: "https://example.test/pr/hx-1".to_string(),
            ..Default::default()
        },
    );
    let session = Session {
        app: "claude".to_string(),
        id: "s-saved".to_string(),
        reset: None,
    };
    state.release = Some(Box::new(Release {
        id: "release-hx".to_string(),
        ts: TicketState {
            status: STATUS_RUNNING.to_string(),
            stage: "release".to_string(),
            sessions: [("release".to_string(), session)].into(),
            ..Default::default()
        },
        ..Default::default()
    }));
}

#[test]
fn continue_on_a_saved_run_resumes_its_release_by_its_session_id() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    stopped_in_release(&w, &o);
    w.lock().merged = true;
    let file = o.run_dir("release-hx").join("release.md");
    w.session(move |p| match p.text == "continue" {
        true => {
            write_file(&file, RELEASED);
            (String::new(), "idle".to_string())
        }
        false => succeed(p),
    });
    let o = Arc::new(o);

    run_tagged(&w, &o);

    let starts = w.called("herdr agent start h-release-hx-release ");
    assert!(
        starts.len() == 1
            && starts[0].contains("--kind claude --pane ")
            && starts[0].contains(" -- --resume s-saved "),
        "the Release's starts = {starts:?}, want one resuming s-saved"
    );
    let pane = o.ticket("release-hx").panes.get("release").cloned();
    assert_eq!(pane, None, "its tab closed once done");
    let prompts = w.called("herdr agent prompt ");
    assert_eq!(prompts.len(), 1, "prompts = {prompts:?}");
    assert!(prompts[0].ends_with(" continue"), "{prompts:?}");
    assert!(run_level(&w, "release resumed: claude (pane"));
    w.await_line("release done: v1.5.0");
}

#[test]
fn a_short_usage_limit_on_the_release_holds_it_and_it_carries_on_at_the_reset() {
    let (w, mut o) = releasing(vec![BdTicket::new("hx-1")]);
    w.lock().epic_labels = label();
    let clock = set_clock(&mut o.cfg, now());
    hits(&w, "release-hx", "release", "idle", CLAUDE);
    let o = Arc::new(o);

    let mut run = spawn_epic(o.clone(), "hx");

    w.await_line("release-hx claude session limit until 3:45pm: release holds (pane");
    assert!(
        !run.finished_within(Duration::from_millis(30)),
        "the Release did not hold"
    );
    assert_eq!(o.ticket("release-hx").limited, "claude");
    let file = o.run_dir("release-hx").join("release.md");
    w.session(move |_| {
        write_file(&file, RELEASED); // the continue
        (String::new(), "idle".to_string())
    });
    *clock.lock().unwrap() = at(25, 15, 47);
    tag(&w, &o, &mut run, "no");
    w.await_line("release-hx claude session limit over: release carries on (pane");
    w.await_line("release done: v1.5.0");
}

#[test]
fn a_release_question_while_away_waits_unparked_with_no_bd_comment() {
    let (w, o) = releasing(vec![BdTicket::new("hx-1")]);
    w.lock().epic_labels = label();
    o.cfg.away.store(true, Ordering::SeqCst);
    w.session(|p| match p.stage == "release" {
        true => (ASKS.to_string(), "idle".to_string()),
        false => succeed(p),
    });
    let o = Arc::new(o);

    let run = spawn_epic(o.clone(), "hx");

    let asked = w.await_event("question in release");
    assert!(matches!(asked.ask, Some(Ask::StageQuestion { .. })));
    assert_eq!(asked.ticket.as_deref(), Some("release-hx"));
    assert!(
        !run.finished_within(Duration::from_millis(30)),
        "the run did not wait on the Release's question"
    );
    assert!(w.called("bd comments add").is_empty());
    assert_eq!(o.ticket("release-hx").status, STATUS_RUNNING);
}

#[test]
fn a_wake_on_the_release_settled_as_park_stops_the_run_and_continue_starts_it_again() {
    let (w, o) = releasing(vec![BdTicket::new("hx-1")]);
    w.lock().epic_labels = label();
    w.session(|p| match p.stage == "release" {
        true => (String::new(), "idle".to_string()), // no result
        false => succeed(p),
    });
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");
    let stuck = w.await_event("stuck in release: went idle without a result");
    let Some(Ask::Wake { pane, .. }) = stuck.ask else {
        panic!("no Wake: {stuck:?}");
    };

    o.answer("release-hx", &pane, Answer::Act(Action::Park));

    run.wait();
    assert!(o.stopping(), "the run ended as done, not stopped");
    w.await_line("release-hx parked: release went idle without a result");
    assert_eq!(o.ticket("release-hx").status, STATUS_PARKED);

    // /continue: its live session watched again, not started anew
    let before = w.calls().len();
    let again = restarted(&w, &o);
    let mut run = spawn_epic(again.clone(), "hx");
    w.await_nth("stuck in release: went idle without a result", 2);
    write_file(&o.run_dir("release-hx").join("release.md"), RELEASED);
    tag(&w, &again, &mut run, "no");
    w.await_line("release done: v1.5.0");
    assert!(w.since(before, "herdr agent start ").is_empty());
}

#[test]
fn a_long_usage_limit_on_the_release_ends_the_run_with_its_session_saved() {
    let (w, mut o) = releasing(vec![BdTicket::new("hx-1")]);
    w.lock().epic_labels = label();
    w.lock().integration = true;
    set_clock(&mut o.cfg, now());
    let weekly = "You've hit your weekly limit · resets Mon 12:00am";
    hits(&w, "release-hx", "release", "idle", weekly);
    let o = Arc::new(o);

    spawn_epic(o.clone(), "hx").wait(); // it ends by itself

    w.await_line("claude weekly limit until Mon 12:00am: sessions saved, panes closed");
    assert!(
        o.stopping() && o.closed(),
        "the run did not end at the limit"
    );
    assert!(w.lock().tabs.is_empty(), "the Release's tab stayed");
    let ts = o.ticket("release-hx");
    assert!(
        ts.tab.is_empty() && ts.panes.is_empty() && !ts.sessions["release"].id.is_empty(),
        "{ts:?}"
    );
}

#[test]
fn the_tag_question_while_away_waits_unparked_with_no_bd_comment() {
    let (w, o) = releasing(vec![BdTicket::new("hx-1")]);
    w.lock().epic_labels = label();
    o.cfg.away.store(true, Ordering::SeqCst);
    let o = Arc::new(o);

    let mut run = spawn_epic(o.clone(), "hx");

    w.await_event("Tag v1.5.0 and push it?");
    assert!(
        !run.finished_within(Duration::from_millis(30)),
        "the run did not wait on the tag Question"
    );
    assert!(w.called("bd comments add").is_empty());
    assert_ne!(o.ticket("release-hx").status, STATUS_PARKED);
    tag(&w, &o, &mut run, "no");
}

#[test]
fn continue_on_a_saved_run_whose_version_pr_is_open_polls_it_again() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    stopped_in_release(&w, &o);
    {
        let mut state = o.state.lock().unwrap();
        let release = state.release.as_mut().unwrap();
        release.version = "v1.5.0".to_string();
        release.ts.pr = VERSION_PR.to_string();
        release.ts.sessions.clear();
    }
    let open = r#"{"state":"OPEN","mergeable":"MERGEABLE"}"#.to_string();
    w.lock().prs.insert(VERSION_PR.to_string(), open);
    let o = Arc::new(o);

    let mut run = spawn_epic(o.clone(), "hx");

    let poll = format!("url={VERSION_PR}");
    wait_until("the version PR polled", || {
        w.called("gh api graphql")
            .iter()
            .any(|c| c.ends_with(&poll))
    });
    assert!(
        w.called("herdr agent start").is_empty(),
        "the Release's Stage ran again"
    );
    w.lock().prs.remove(VERSION_PR); // merged
    w.lock().merged = true;
    tag(&w, &o, &mut run, "no");
    w.await_line("version PR #version merged");
}

#[test]
fn continue_on_a_saved_run_whose_release_is_tagged_asks_no_tag_question() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    stopped_in_release(&w, &o);
    {
        let mut state = o.state.lock().unwrap();
        let release = state.release.as_mut().unwrap();
        release.version = "v1.5.0".to_string();
        release.ts.pr = VERSION_PR.to_string();
        release.ts.sessions.clear();
        release.commit = "m3rg3d".to_string();
        release.tagged = true;
    }
    let o = Arc::new(o);

    spawn_epic(o.clone(), "hx").wait();

    assert!(
        w.events().iter().all(|e| e.ask.is_none()),
        "{:?}",
        w.events()
    );
    assert!(w.called("git tag").is_empty());
}

#[test]
fn a_release_result_without_its_version_wakes() {
    let (w, o) = releasing(vec![BdTicket::new("hx-1")]);
    w.lock().epic_labels = label();
    w.session(|p| match p.stage == "release" {
        true => (
            format!("STATUS: done\nPR: {VERSION_PR}\n"),
            "idle".to_string(),
        ),
        false => succeed(p),
    });
    let o = Arc::new(o);

    let _run = spawn_epic(o.clone(), "hx");

    let stuck = w.await_event("stuck in release: finished without a VERSION line");
    assert!(matches!(stuck.ask, Some(Ask::Wake { .. })), "{stuck:?}");
}

#[test]
fn a_tag_that_fails_says_so_and_asks_again() {
    let (w, o) = releasing(vec![BdTicket::new("hx-1")]);
    w.lock().epic_labels = label();
    w.fail_once("git push origin v1.5.0", "rejected");
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");
    w.await_event("Tag v1.5.0 and push it?");

    o.answer("release-hx", "", Answer::Prompt("yes".to_string()));

    w.await_nth("Tag v1.5.0 and push it?", 2);
    assert!(run_level(&w, "tag v1.5.0 failed: "));
    assert!(!o.state.lock().unwrap().release.clone().unwrap().tagged);
    // the tag the failed push left goes, or every git tag after it fails
    assert_eq!(w.called("git tag -d"), ["git tag -d v1.5.0"]);

    let before = w.calls().len();
    tag(&w, &o, &mut run, "yes");
    assert_eq!(
        w.since(before, "git "),
        [
            "git fetch origin HEAD",
            "git tag v1.5.0 m3rg3d",
            "git push origin v1.5.0"
        ]
    );
    assert!(o.state.lock().unwrap().release.clone().unwrap().tagged);
}

/// The version PR open on head `a`, mergeable, its one check in `state`.
fn open_version_pr(w: &World, state: &str) {
    let pr = json!({"state": "OPEN", "mergeable": "MERGEABLE", "headRefOid": "a",
        "statusCheckRollup": {"commit": {"oid": "a"},
            "contexts": {"nodes": [{"context": "ci", "state": state}]}}});
    w.lock().prs.insert(VERSION_PR.to_string(), pr.to_string());
}

/// Waits for two more polls of the version PR: the first of them is done.
fn polled(w: &World) {
    let poll = format!("url={VERSION_PR}");
    let count = || {
        let polls = w.called("gh api graphql");
        polls.iter().filter(|c| c.ends_with(&poll)).count()
    };
    let before = count();
    wait_until("the version PR polled", || count() >= before + 2);
}

/// Lets the version PR's head, once the poll saw it, hold its quiet
/// minute: the next poll may merge it, and a merge ends the polls.
fn settle(w: &World, clock: &Mutex<DateTime<Local>>) {
    polled(w);
    *clock.lock().unwrap() += TimeDelta::seconds(60);
}

/// The version PR's orqa:no-review, as gh is asked for it.
fn labelled() -> String {
    format!("gh pr edit {VERSION_PR} --add-label orqa:no-review")
}

#[test]
fn with_agent_merge_off_the_version_pr_is_labelled_but_never_merged_and_the_tag_question_is_asked()
{
    let (w, mut o) = releasing(vec![BdTicket::new("hx-1")]);
    w.lock().epic_labels = label();
    let clock = set_clock(&mut o.cfg, now());
    open_version_pr(&w, "SUCCESS");
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");

    w.await_line("version PR #version opened");
    settle(&w, &clock);
    polled(&w);
    assert_eq!(w.called("gh pr edit"), [labelled()], "whatever its files");
    assert!(w.called("gh pr merge").is_empty(), "merged with it off");

    w.lock().prs.remove(VERSION_PR); // merged by hand
    w.await_event("Tag v1.5.0 and push it?");
    tag(&w, &o, &mut run, "no");
}

/// An Epic run under Agent merge up to its version PR, open on a green,
/// mergeable head: the run and its clock.
fn agent_merging(
    w: &Arc<World>,
    mut o: Orchestrator,
) -> (Arc<Orchestrator>, Running, Arc<Mutex<DateTime<Local>>>) {
    w.lock().epic_labels = label();
    agent_merge(w, &["coderabbit"]);
    let clock = set_clock(&mut o.cfg, now());
    open_version_pr(w, "SUCCESS");
    let o = Arc::new(o);
    let run = spawn_epic(o.clone(), "hx");
    w.await_line("version PR #version opened");
    (o, run, clock)
}

#[test]
fn under_agent_merge_the_version_pr_is_labelled_merged_on_green_and_tagged_with_no_question() {
    let (w, o) = releasing(vec![BdTicket::new("hx-1")]);
    let (o, mut run, clock) = agent_merging(&w, o);
    open_version_pr(&w, "FAILURE");
    settle(&w, &clock);
    polled(&w);
    assert!(w.called("gh pr merge").is_empty(), "merged before green");

    let before = w.calls().len();
    open_version_pr(&w, "SUCCESS");
    run.wait();

    assert_eq!(w.called("gh pr edit"), [labelled()]);
    let merge = format!(
        "gh pr merge {VERSION_PR} --squash --delete-branch --match-head-commit a --repo o/r"
    );
    assert_eq!(w.called("gh pr merge"), [merge]);
    assert_eq!(
        w.since(before, "git "),
        [
            "git branch -D release-hx",
            "git fetch origin HEAD",
            "git tag v1.5.0 m3rg3d",
            "git push origin v1.5.0"
        ]
    );
    let lines = w.lines();
    let line = |text: &str| {
        lines
            .iter()
            .position(|l| l == text)
            .unwrap_or_else(|| panic!("no line {text:?}:\n{}", lines.join("\n")))
    };
    assert!(line("version PR #version merged by Orqadence") < line("version PR #version merged"));
    assert!(line("version PR #version merged") < line("tagged v1.5.0 and pushed"));
    let asked: Vec<_> = w.events().into_iter().filter_map(|e| e.ask).collect();
    assert!(
        !asked.iter().any(|ask| matches!(ask, Ask::Tag { .. })),
        "the tag Question: {asked:?}"
    );
    assert!(!o.stopping(), "the run stopped rather than ended");
    let release = o.state.lock().unwrap().release.clone().unwrap();
    assert!(release.tagged && release.commit == "m3rg3d", "{release:?}");
}

/// A /stop-work while the poll reads the version PR the Orchestrator
/// merged pushes no tag: /continue asks the tag Question.
#[test]
fn a_stop_while_the_merged_version_pr_is_read_pushes_no_tag() {
    let (w, o) = releasing(vec![BdTicket::new("hx-1")]);
    let orq: Arc<Mutex<Weak<Orchestrator>>> = Arc::default();
    let hooked = orq.clone();
    let poll = format!("url={VERSION_PR}");
    w.hook(move |_, argv| {
        let read = argv.starts_with(&["gh", "api", "graphql"]) && argv.join(" ").ends_with(&poll);
        if let Some(o) = hooked
            .lock()
            .unwrap()
            .upgrade()
            .filter(|o| read && o.ticket("release-hx").merge_asked)
        {
            o.stop();
        }
        None
    });
    let (o, mut run, clock) = agent_merging(&w, o);
    *orq.lock().unwrap() = Arc::downgrade(&o);

    settle(&w, &clock);
    run.wait();
    assert_eq!(w.called("gh pr merge").len(), 1);
    assert!(w.called("git tag").is_empty(), "tagged after the stop");
    let release = o.state.lock().unwrap().release.clone().unwrap();
    assert!(!release.tagged && release.commit == "m3rg3d", "{release:?}");
}

/// A /stop-work during the fetch before the unasked tag tags nothing:
/// /continue asks the tag Question.
#[test]
fn a_stop_during_the_fetch_before_the_unasked_tag_tags_nothing() {
    let (w, o) = releasing(vec![BdTicket::new("hx-1")]);
    let orq: Arc<Mutex<Weak<Orchestrator>>> = Arc::default();
    let hooked = orq.clone();
    w.hook(move |_, argv| {
        if argv.starts_with(&["git", "fetch"]) {
            if let Some(o) = hooked.lock().unwrap().upgrade() {
                o.stop();
            }
        }
        None
    });
    let (o, mut run, clock) = agent_merging(&w, o);
    *orq.lock().unwrap() = Arc::downgrade(&o);

    settle(&w, &clock);
    run.wait();
    assert_eq!(w.called("gh pr merge").len(), 1);
    assert!(w.called("git tag").is_empty(), "tagged after the stop");
    let release = o.state.lock().unwrap().release.clone().unwrap();
    assert!(!release.tagged, "{release:?}");
}

#[test]
fn a_refused_merge_of_the_version_pr_says_why_and_falls_back_to_the_tag_question() {
    let (w, o) = releasing(vec![BdTicket::new("hx-1")]);
    let refusal = "the base branch policy prohibits the merge.";
    w.fail_once("gh pr merge", refusal);
    let (o, mut run, clock) = agent_merging(&w, o);

    settle(&w, &clock);
    w.await_line(&format!("version PR #version not merged: {refusal}"));
    polled(&w);
    assert_eq!(w.called("gh pr merge").len(), 1, "asked again");
    assert!(w.called("git tag").is_empty());

    w.lock().prs.remove(VERSION_PR); // merged by hand
    w.await_event("Tag v1.5.0 and push it?");
    tag(&w, &o, &mut run, "yes");
    assert_eq!(w.called("git tag"), ["git tag v1.5.0 m3rg3d"]);
    assert!(run_level(&w, "tagged v1.5.0 and pushed"));
}

#[test]
fn a_tag_push_that_fails_under_agent_merge_says_why_and_asks_the_tag_question() {
    let (w, o) = releasing(vec![BdTicket::new("hx-1")]);
    w.fail_once("git push origin v1.5.0", "rejected");
    let (o, mut run, clock) = agent_merging(&w, o);

    settle(&w, &clock);
    w.await_event("Tag v1.5.0 and push it?");
    assert_eq!(w.called("gh pr merge").len(), 1);
    assert!(
        w.lines().iter().any(|l| l.ends_with("rejected")),
        "git's message"
    );
    assert!(run_level(&w, "tag v1.5.0 failed: "));
    assert_eq!(w.called("git tag -d"), ["git tag -d v1.5.0"]);
    assert!(!o.state.lock().unwrap().release.clone().unwrap().tagged);

    tag(&w, &o, &mut run, "yes");
    assert!(o.state.lock().unwrap().release.clone().unwrap().tagged);
}

#[test]
fn a_version_pr_gh_does_not_label_says_so_and_still_merges_on_green() {
    let (w, o) = releasing(vec![BdTicket::new("hx-1")]);
    w.fail_once("gh pr edit", "HTTP 502");
    w.fail_once("gh label create", "HTTP 502");
    let (_o, mut run, clock) = agent_merging(&w, o);

    settle(&w, &clock);
    run.wait();
    assert!(run_level(
        &w,
        "version PR #version not labelled orqa:no-review: "
    ));
    assert_eq!(w.called("gh pr merge").len(), 1);
    assert!(run_level(&w, "tagged v1.5.0 and pushed"));
}
