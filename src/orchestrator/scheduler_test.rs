use super::app::{set_count, set_switch, MAX_PR_SESSIONS, MAX_TICKETS, REBASE_AUTO};
use super::question_test::ASKS;
use super::stage::{Answer, Ask, Config, Orchestrator, AWAY};
use super::state::{
    acquire_lock, load_state, lock_holder, STATUS_MERGED, STATUS_PARKED, STATUS_PR_OPEN,
    STATUS_RUNNING,
};
use super::world::{
    new_world, spawn_epic, succeed, wait_until, working, BdTicket, Prompt, Running, World,
};
use super::write_file;
use std::fs;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Runs the Orchestrator until it returns, as /start-epic does.
pub(super) fn run_epic(o: &Arc<Orchestrator>) {
    let mut run = spawn_epic(o.clone(), "hx");
    assert!(
        run.finished_within(Duration::from_secs(10)),
        "the Orchestrator never finished the Epic"
    );
    run.wait();
    o.wait_in_flight();
}

pub(super) fn with_deps(id: &str, deps: &[&str]) -> BdTicket {
    BdTicket {
        deps: deps.iter().map(|d| d.to_string()).collect(),
        ..BdTicket::new(id)
    }
}

#[test]
fn scheduler_runs_every_ready_ticket_but_never_more_than_max_at_once() {
    for max in [3, 2] {
        let tickets = (1..=5).map(|i| BdTicket::new(&format!("hx-{i}"))).collect();
        let (w, o) = new_world(tickets);
        set_count(&w.repo, &MAX_TICKETS, Some(max)).unwrap();
        w.lock().merged = true;
        w.session(|p| {
            thread::sleep(Duration::from_millis(2)); // long enough for Tickets to overlap
            succeed(p)
        });
        let o = Arc::new(o);

        run_epic(&o);

        assert_eq!(w.called("bd worktree create").len(), 5, "Tickets run");
        let peak = w.lock().peak;
        assert_eq!(
            peak, max,
            "most Tickets in the Pipeline at once, want exactly max_tickets {max}"
        );
        w.await_line("Epic done, every Ticket closed");
    }
}

/// A Ticket thread that dies gives its slot back, so the Ticket is
/// resumed on the next tick instead of holding the Pipeline forever.
#[test]
fn a_ticket_thread_that_panics_frees_its_slot() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().merged = true;
    let died = AtomicBool::new(false);
    w.session(move |p| {
        if p.stage == "implement" && !died.swap(true, Ordering::SeqCst) {
            panic!("the fake world failed inside the Ticket thread");
        }
        succeed(p)
    });
    let o = Arc::new(o);

    let mut run = spawn_epic(o.clone(), "hx");
    // Resumed, the Ticket watches its live pane, idle without a result: a Wake.
    w.await_line("hx-1 stuck in implement: went idle without a result (pane 1-1)");
    o.command("retry-hx-1");
    assert!(
        run.finished_within(Duration::from_secs(10)),
        "the Epic never finished: the dead Ticket's slot was not given back"
    );
    run.wait();
    let joined = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| o.wait_in_flight()));
    assert!(joined.is_err(), "the Ticket thread's panic was lost");

    assert!(
        o.active.lock().unwrap().is_empty(),
        "a dead Ticket still holds a slot"
    );
    assert_eq!(
        o.ticket("hx-1").status,
        STATUS_MERGED,
        "the Ticket was not resumed"
    );
    assert_eq!(
        w.called("herdr agent start h-hx-1-implement").len(),
        2,
        "want a fresh Implement session after the thread died"
    );
}

#[test]
fn blocked_ticket_starts_only_after_its_dependency_is_merged_and_closed() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1"), with_deps("hx-2", &["hx-1"])]);
    w.lock().merged = true;
    let o = Arc::new(o);

    run_epic(&o);

    w.await_line("hx-1 merged, Ticket closed");
    let order = w.calls().join("\n");
    let closed = order.find("bd close hx-1");
    let started = order.find(&format!(
        "bd worktree create {}",
        o.worktree("hx-2").display()
    ));
    assert!(
        matches!((closed, started), (Some(c), Some(s)) if s >= c),
        "hx-2 must start after hx-1 is closed (close at {closed:?}, start at {started:?})"
    );
    for want in [
        format!("bd worktree remove {}", o.worktree("hx-1").display()),
        "git branch -D hx-1".to_string(),
    ] {
        assert_eq!(w.called(&want).len(), 1, "merge cleanup missing {want:?}");
    }
    assert_eq!(o.ticket("hx-1").status, STATUS_MERGED, "hx-1 status");
}

#[test]
fn one_run_per_target_repo_but_a_stale_lock_does_not_block_a_restart() {
    let repo = crate::tempdir::TempDir::new();
    let release = acquire_lock(repo.path()).unwrap(); // a live Orchestrator: this process
    let err = acquire_lock(repo.path())
        .map(|_| ())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("a run is live"),
        "second lock = {err}, want a refusal"
    );
    assert_eq!(
        lock_holder(repo.path()),
        std::process::id(),
        "LockHolder, want this process"
    );

    drop(release);
    fs::write(repo.path().join(".orqadence-local/lock"), "999999").unwrap(); // a killed Orchestrator's stale lock
    assert!(
        acquire_lock(repo.path()).is_ok(),
        "a stale lock must not block a restart"
    );
}

#[test]
fn closed_pr_parks_the_ticket_and_conflict_is_reported_exactly_once() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1"), BdTicket::new("hx-2")]);
    {
        let mut inner = w.lock();
        inner.prs.insert(
            "https://example.test/pr/hx-1".to_string(),
            r#"{"state":"CLOSED","mergeable":"UNKNOWN"}"#.to_string(),
        );
        inner.prs.insert(
            "https://example.test/pr/hx-2".to_string(),
            r#"{"state":"OPEN","mergeable":"CONFLICTING"}"#.to_string(),
        );
    }
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");

    w.await_line("hx-1 parked: PR #hx-1 closed without merging");
    w.await_line("hx-2 PR #hx-2 conflicts with main, /rebase resolves it");
    thread::sleep(Duration::from_millis(30)); // many more polls
    o.stop();
    run.wait();
    o.wait_in_flight();

    let conflicts = w
        .lines()
        .iter()
        .filter(|l| l.contains("conflicts with main"))
        .count();
    assert_eq!(
        conflicts, 1,
        "conflict reported {conflicts} times, want once"
    );
    assert!(!started(&w, "h-hx-2-rebase"), "rebase_auto is off");
    assert_eq!(
        o.ticket("hx-1").status,
        STATUS_PARKED,
        "hx-1 status, want parked"
    );
    assert!(
        w.called("bd close").is_empty(),
        "no Ticket may be closed without a merge"
    );
}

#[test]
fn each_poll_asks_gh_once_per_open_pr() {
    let tickets = ["hx-1", "hx-2", "hx-3"].map(BdTicket::new).to_vec();
    let (w, o) = new_world(tickets);
    for ticket in ["hx-1", "hx-2"] {
        o.update(ticket, |ts| {
            ts.status = STATUS_PR_OPEN.to_string();
            ts.pr = format!("https://example.test/pr/{ticket}");
        });
    }
    o.update("hx-3", |_| {}); // running: no PR yet

    o.poll_merges();
    let gh = w.called("gh ");
    assert_eq!(gh.len(), 2, "{gh:#?}");
    for (call, ticket) in gh.iter().zip(["hx-1", "hx-2"]) {
        assert!(
            call.starts_with("gh api graphql -f query=")
                && call.ends_with(&format!(" -f url=https://example.test/pr/{ticket}")),
            "{call}"
        );
    }
}

#[test]
fn killed_run_resumes_at_the_right_stage_without_redoing_finished_ones() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().merged = true;
    let o = Arc::new(o);
    // The first Orchestrator is stopped while Review round 1 is mid-flight.
    let kill = o.clone();
    w.session(move |p| {
        if p.stage == "review" {
            kill.stop.store(true, Ordering::SeqCst);
            return (String::new(), "working".to_string());
        }
        succeed(p)
    });
    let _ = o.run("hx");
    o.wait_in_flight();
    let ts = o.ticket("hx-1");
    assert!(
        ts.status == STATUS_RUNNING && ts.stage == "review" && ts.round == 1,
        "state when killed = {ts:?}"
    );

    // A new process: fresh Orchestrator, state loaded from the file.
    let old_pane = o.ticket("hx-1").panes["review"].clone();
    let old = w.lock().agents.get(&old_pane).cloned();
    assert_eq!(
        old.as_deref(),
        Some("working"),
        "restart fixture must leave Review alive"
    );
    let seen = w.clone();
    w.session(move |p| {
        if p.stage == "review" {
            let alive = seen.lock().agents.contains_key(&old_pane);
            assert!(
                !alive,
                "resumed Review was prompted while its old session was still alive"
            );
        }
        succeed(p)
    });
    let state = load_state(&w.repo).unwrap();
    let mut cfg = Config::for_tests(w.clone(), &w.repo, &w.home);
    cfg.events = o.cfg.events.clone();
    let resumed = Arc::new(Orchestrator::with_state(cfg, state));
    let before = w.called("herdr agent start").len();
    // The live Review session is watched, not replaced; once it goes idle
    // without a result the Ticket Wakes, and retry starts Review afresh.
    let mut run = spawn_epic(resumed.clone(), "hx");
    let old_pane = resumed.ticket("hx-1").panes["review"].clone();
    w.lock().agents.insert(old_pane, "idle".to_string());
    w.await_line("hx-1 stuck in review 1: went idle without a result");
    resumed.command("retry-hx-1");
    run.wait();
    resumed.wait_in_flight();

    let stages: Vec<String> = w.called("herdr agent start")[before..]
        .iter()
        .map(|call| call.split_whitespace().nth(3).unwrap().to_string())
        .collect();
    assert_eq!(
        stages,
        ["h-hx-1-review", "h-hx-1-debate", "h-hx-1-fix"],
        "sessions after resume; Implement was done and Review starts again from its beginning"
    );
    assert_eq!(
        w.called("bd worktree create").len(),
        1,
        "worktree created more than once"
    );
}

#[test]
fn stop_exits_with_state_saved_and_leaves_panes_alone() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.session(|_| (String::new(), "idle".to_string())); // hx-1 holds on a Wake
    let o = Arc::new(o);
    // Hold the scheduler in an external call while the Ticket consumes stop
    // and exits. It must not resume that now-inactive Ticket when the call
    // returns, even though its persisted state is deliberately still running.
    let armed = Arc::new(AtomicBool::new(false));
    let stopped = Arc::new(AtomicBool::new(false));
    let (arm, run) = (armed.clone(), o.clone());
    w.hook(move |_, argv| {
        if argv.join(" ").starts_with("bd list")
            && !stopped.load(Ordering::SeqCst)
            && arm.load(Ordering::SeqCst)
        {
            stopped.store(true, Ordering::SeqCst);
            run.stop();
            run.wait_in_flight();
        }
        None
    });
    let mut run = spawn_epic(o.clone(), "hx");
    w.await_line("hx-1 stuck in implement");

    armed.store(true, Ordering::SeqCst);
    assert!(
        run.finished_within(Duration::from_secs(5)),
        "Run after stop never returned"
    );
    run.wait();
    o.wait_in_flight();

    assert!(
        w.called("herdr pane close").len() + w.called("herdr tab close").len() == 0,
        "stop must leave live panes alone; calls:\n{}",
        w.calls().join("\n")
    );
    assert_eq!(
        w.called("herdr agent start h-hx-1-implement").len(),
        1,
        "scheduler restarted a stopped Ticket"
    );
    let saved = load_state(&w.repo).unwrap();
    let ts = saved.tickets.get("hx-1");
    assert!(
        ts.is_some_and(|ts| ts.status == STATUS_RUNNING && ts.stage == "implement"),
        "saved state = {ts:?}"
    );
}

#[test]
fn retry_unparks_a_parked_ticket() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().merged = true;
    w.session(|_| (String::new(), "idle".to_string()));
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");

    w.await_line("hx-1 stuck in implement");
    o.command("park-hx-1");
    w.await_line("hx-1 parked: implement went idle without a result");
    w.session(succeed);
    o.command("retry-hx-1");
    run.wait();
    o.wait_in_flight();
    assert_eq!(
        o.ticket("hx-1").status,
        STATUS_MERGED,
        "want the Ticket to run to a merge after the retry"
    );
}

/// The prompt of the first session of `stage`, kept as the world starts it.
fn first_prompt(w: &World, stage: &'static str) -> Arc<Mutex<Option<Prompt>>> {
    let prompt: Arc<Mutex<Option<Prompt>>> = Default::default();
    let seen = prompt.clone();
    w.session(move |p| {
        if p.stage == stage {
            seen.lock().unwrap().get_or_insert_with(|| p.clone());
        }
        succeed(p)
    });
    prompt
}

/// /rebase on a PR the last poll saw conflict: a fresh Rebase session in
/// the kept worktree, fed its Stage skill and the PR; the Ticket stays
/// pr-open.
#[test]
fn rebase_command_on_a_conflicting_pr_starts_rebase_in_the_kept_worktree() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().prs.insert(
        "https://example.test/pr/hx-1".to_string(),
        r#"{"state":"OPEN","mergeable":"CONFLICTING"}"#.to_string(),
    );
    let prompt = first_prompt(&w, "rebase");
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");
    w.await_line("hx-1 PR #hx-1 conflicts with main, /rebase resolves it");

    o.command("rebase-hx-1");
    w.await_line("hx-1 rebasing PR #hx-1");
    w.await_line("hx-1 rebased PR #hx-1");
    o.stop();
    run.wait();
    o.wait_in_flight();

    let text = prompt.lock().unwrap().as_ref().map(|p| p.text.clone());
    let text = text.unwrap_or_default();
    for want in [
        "# Rebase Stage",
        "- PR: https://example.test/pr/hx-1",
        "- Default branch: origin/main",
    ] {
        assert!(text.contains(want), "rebase prompt lacks {want:?}:\n{text}");
    }
    let tabs = w.called("herdr tab create");
    assert!(
        tabs.len() == 2 && tabs[1].contains(&format!("--cwd {}", o.worktree("hx-1").display())),
        "rebase must reopen a Ticket tab in the kept worktree: {tabs:?}"
    );
    assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN);
}

/// /rebase goes by the last poll only: a conflict reported, then UNKNOWN
/// (as GitHub says right after a push), refuses it.
#[test]
fn rebase_command_after_conflicting_then_unknown_is_refused() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let pr = "https://example.test/pr/hx-1".to_string();
    w.lock().prs.insert(
        pr.clone(),
        r#"{"state":"OPEN","mergeable":"CONFLICTING"}"#.to_string(),
    );
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");
    w.await_line("hx-1 PR #hx-1 conflicts with main, /rebase resolves it");
    w.lock()
        .prs
        .insert(pr, r#"{"state":"OPEN","mergeable":"UNKNOWN"}"#.to_string());
    thread::sleep(Duration::from_millis(30)); // many more polls

    o.command("rebase-hx-1");
    w.await_line("hx-1 refused: PR #hx-1 does not conflict with main");
    o.stop();
    run.wait();
    o.wait_in_flight();
    assert!(w.called("herdr agent start h-hx-1-rebase").is_empty());
}

/// After a restart `conflicting` starts false: the run polls before it
/// takes a command, so a /rebase sent at once still sees the conflict.
#[test]
fn rebase_command_right_after_a_restart_sees_the_conflict() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let pr = "https://example.test/pr/hx-1".to_string();
    w.lock().prs.insert(
        pr.clone(),
        r#"{"state":"OPEN","mergeable":"CONFLICTING"}"#.to_string(),
    );
    o.update("hx-1", |ts| {
        ts.status = STATUS_PR_OPEN.to_string();
        ts.pr = pr;
        ts.conflict = true; // saved; conflicting is not
    });
    let kept = o.worktree("hx-1").display().to_string();
    let create = ["bd", "worktree", "create", &kept];
    o.cfg.tools.run(&o.cfg.repo, &create).unwrap();
    o.command("rebase-hx-1");
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");

    w.await_line("hx-1 rebased PR #hx-1");
    o.stop();
    run.wait();
    o.wait_in_flight();
}

/// /address-pr-comments: a fresh session fed the PR and gh's view of its
/// comments, its pr comments line naming the Shipped default; the Ticket
/// stays pr-open.
#[test]
fn address_pr_comments_command_starts_it_with_the_pr_and_the_gh_json() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().prs.insert(
        "https://example.test/pr/hx-1".to_string(),
        r#"{"state":"OPEN","mergeable":"MERGEABLE","reviews":[{"body":"rename this"}]}"#
            .to_string(),
    );
    let prompt = first_prompt(&w, "address-pr-comments");
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");
    w.await_line("hx-1 PR #hx-1 opened");

    o.command("address-pr-comments-hx-1");
    w.await_line("hx-1 addressed PR #hx-1");
    o.stop();
    run.wait();
    o.wait_in_flight();

    let text = prompt.lock().unwrap().as_ref().map(|p| p.text.clone());
    let text = text.unwrap_or_default();
    for want in [
        "# Address PR comments Stage",
        "- PR: https://example.test/pr/hx-1",
        "- PR metadata (gh JSON): ",
        "rename this",
        "   Use the orqa-address-pr-comments skill for steps 2 to 5",
    ] {
        assert!(
            text.contains(want),
            "address-pr-comments prompt lacks {want:?}:\n{text}"
        );
    }
    let view = "gh pr view https://example.test/pr/hx-1 --json reviews,comments,statusCheckRollup";
    assert!(!w.called(view).is_empty(), "no failing checks asked of gh");
    assert_eq!(
        o.ticket("hx-1").status,
        STATUS_PR_OPEN,
        "status after address-pr-comments, want it still pr-open"
    );
}

/// The Address PR comments prompt of hx-1, its PR open in the run `start`
/// spawns over the world.
fn address_prompt(
    (w, o): (Arc<World>, Orchestrator),
    start: impl Fn(&Arc<Orchestrator>) -> Running,
) -> String {
    let prompt = first_prompt(&w, "address-pr-comments");
    let o = Arc::new(o);
    let mut run = start(&o);
    w.await_line("hx-1 PR #hx-1 opened");

    o.command("address-pr-comments-hx-1");
    w.await_line("hx-1 addressed PR #hx-1");
    o.stop();
    run.wait();
    o.wait_in_flight();
    let text = prompt.lock().unwrap().as_ref().map(|p| p.text.clone());
    text.unwrap_or_default()
}

/// Address PR comments is fed the Epic's other open children, id and title
/// each: a PR comment may ask for the work one of them does. Not the Ticket
/// itself, nor a closed one.
#[test]
fn address_pr_comments_inputs_name_the_epics_other_open_tickets() {
    let world = new_world(vec![
        BdTicket::new("hx-1"),
        with_deps("hx-2", &["hx-1"]), // waits on hx-1's merge: still open
        BdTicket::new("hx-3"),
    ]);
    world.0.lock().tickets[2].status = "closed".to_string();
    let text = address_prompt(world, |o| spawn_epic(o.clone(), "hx"));
    let want = "- Other open Tickets: \n  - hx-2: Ticket hx-2\n- PR metadata (gh JSON): ";
    assert!(text.contains(want), "prompt lacks {want:?}:\n{text}");
}

/// A Ticket run: the queue's, and the Ticket's own Epic's children that
/// were never queued.
#[test]
fn address_pr_comments_inputs_name_the_queue_and_the_tickets_epics_children() {
    let lone = BdTicket {
        no_epic: true,
        ..with_deps("hx-3", &["hx-1"])
    };
    let world = new_world(vec![BdTicket::new("hx-1"), BdTicket::new("hx-2"), lone]);
    let text = address_prompt(world, |o| {
        assert!(o.enqueue(&["hx-1".to_string(), "hx-3".to_string()]));
        spawn_epic(o.clone(), "")
    });
    let want = "- Other open Tickets: \n  - hx-3: Ticket hx-3\n  - hx-2: Ticket hx-2\n- PR ";
    assert!(text.contains(want), "prompt lacks {want:?}:\n{text}");
}

/// Without bd's list the session could delete what another Ticket builds:
/// the run does not start, as when gh's view fails.
#[test]
fn address_pr_comments_does_not_start_without_the_other_open_tickets() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let down = AtomicBool::new(false);
    w.hook(move |_, argv| {
        // bd goes down once Address PR comments has gh's view
        let cmd = argv.join(" ");
        if cmd.starts_with("gh pr view") {
            down.store(true, Ordering::SeqCst);
        }
        (cmd.starts_with("bd list") && down.load(Ordering::SeqCst))
            .then(|| Err("dolt: database is locked".to_string()))
    });
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");
    w.await_line("hx-1 PR #hx-1 opened");

    o.command("address-pr-comments-hx-1");
    w.await_line("hx-1 address pr comments failed: bd list ");
    o.stop();
    run.wait();
    o.wait_in_flight();
    assert!(!started(&w, "h-hx-1-address-pr-comments"));
    assert_eq!(o.ticket("hx-1").address_runs, 0, "a run that never started");
}

/// While Address PR comments runs, its Ticket says which run of the cap it
/// is, for TICKETS, and RECENT says it as it starts; once it is done,
/// nothing.
#[test]
fn address_pr_comments_says_its_run_of_the_cap_while_it_runs() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().prs.insert(
        "https://example.test/pr/hx-1".to_string(),
        r#"{"state":"OPEN","mergeable":"MERGEABLE"}"#.to_string(),
    );
    let o = Arc::new(o);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (orq, saw) = (Arc::downgrade(&o), seen.clone());
    w.session(move |p| {
        if let (Some(o), "address-pr-comments") = (orq.upgrade(), p.stage.as_str()) {
            saw.lock().unwrap().push(o.ticket("hx-1").pr_work);
        }
        succeed(p)
    });
    let mut run = spawn_epic(o.clone(), "hx");
    w.await_line("hx-1 PR #hx-1 opened");

    o.command("address-pr-comments-hx-1");
    w.await_line("hx-1 addressed PR #hx-1");
    o.stop();
    run.wait();
    o.wait_in_flight();
    assert_eq!(*seen.lock().unwrap(), ["comments 1/3"]);
    assert_eq!(o.ticket("hx-1").pr_work, "");
    let lines = w.lines();
    let at = |want: &str| lines.iter().position(|l| l.starts_with(want));
    let (doing, started) = (
        at("hx-1 addressing PR #hx-1 (comments 1/3)"),
        at("hx-1 address pr comments started: "),
    );
    assert!(
        matches!((doing, started), (Some(d), Some(s)) if d < s),
        "{lines:#?}"
    );
}

#[test]
fn epic_without_tickets_is_an_error_not_a_done_epic() {
    let (w, o) = new_world(vec![]); // a mistyped Epic id: bd lists no children
    let o = Arc::new(o);
    let err = o.run("hx-typo").unwrap_err();
    assert!(
        err.contains("no Tickets"),
        "Run = {err}, want an error naming the empty Epic"
    );
    for line in w.lines() {
        assert!(
            !line.contains("Epic done"),
            "reported {line:?} for an Epic with no Tickets"
        );
    }
}

/// Every command answer names what happened, and a run-level failure shows.
#[test]
fn command_lines_say_what_was_refused_ignored_or_failed() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.fail_once("bd list", "dolt: database is locked");
    w.fail_once("bd ready", "dolt: database is locked");
    let gh_down = AtomicBool::new(true);
    w.hook(move |_, argv| {
        // Address PR comments' own gh view, not the merge poll's
        if argv.join(" ").starts_with("gh pr view") && gh_down.swap(false, Ordering::SeqCst) {
            return Some(Err("gh: boom".to_string()));
        }
        None
    });
    w.session(|p| {
        if p.stage == "address-pr-comments" {
            return (String::new(), "idle".to_string());
        }
        succeed(p)
    });
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");

    w.await_line("bd list failed: ");
    w.await_line("bd ready failed: ");
    w.await_line("hx-1 PR #hx-1 opened");
    o.command("retry-hx-9");
    w.await_line("hx-9 refused: not a Ticket of this run");
    o.command("park-hx-1");
    w.await_line("hx-1 ignored: not waiting on a Wake");
    o.command("address-pr-comments-hx-9");
    w.await_line("hx-9 address pr comments refused: no open PR");
    o.command("rebase-hx-9");
    w.await_line("hx-9 rebase refused: no open PR");
    o.command("address-pr-comments-hx-1");
    w.await_line("hx-1 address pr comments failed: ");
    o.command("address-pr-comments-hx-1");
    w.await_line("hx-1 stuck in address pr comments: went idle without a result (pane 1-1)");
    o.command("park-hx-1");
    w.await_line(
        "hx-1 address pr comments gave up: address pr comments went idle without a result",
    );
    o.stop();
    run.wait();
    o.wait_in_flight();
}

/// A Ticket that joins the queue while the scheduler lists the queue it
/// had, every Ticket of it closed, still runs: the run ends only on the
/// queue it listed, and once it has ended enqueue refuses.
#[test]
fn a_ticket_run_never_ends_over_a_ticket_enqueued_while_it_lists() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1"), BdTicket::new("hx-2")]);
    w.lock().tickets[0].status = "closed".to_string();
    w.lock().merged = true;
    let o = Arc::new(o);
    assert!(o.enqueue(&["hx-1".to_string()]));
    let (armed, run) = (AtomicBool::new(true), o.clone());
    w.hook(move |_, argv| {
        if argv.join(" ").starts_with("bd list --id hx-1 ") && armed.swap(false, Ordering::SeqCst) {
            assert!(run.enqueue(&["hx-2".to_string()]));
        }
        None
    });
    let mut run = spawn_epic(o.clone(), "");
    run.wait();
    o.wait_in_flight();
    assert_eq!(w.called("bd close hx-2 ").len(), 1, "hx-2 never ran");
    w.await_line("Ticket run done, every Ticket closed");
    assert!(
        !o.enqueue(&["hx-3".to_string()]),
        "a Ticket joined a finished run"
    );
    assert_eq!(o.state.lock().unwrap().queue, ["hx-1", "hx-2"]);
}

const CONFLICTING: &str = r#"{"state":"OPEN","mergeable":"CONFLICTING"}"#;
pub(crate) const MERGEABLE: &str = r#"{"state":"OPEN","mergeable":"MERGEABLE"}"#;

/// `ticket`'s PR open, as gh shows it in `pr`, its worktree kept.
pub(crate) fn pr_open(w: &World, o: &Orchestrator, ticket: &str, pr: &str) {
    let url = format!("https://example.test/pr/{ticket}");
    w.lock().prs.insert(url.clone(), pr.to_string());
    o.update(ticket, |ts| {
        ts.status = STATUS_PR_OPEN.to_string();
        ts.pr = url;
    });
    let kept = o.worktree(ticket).display().to_string();
    let create = ["bd", "worktree", "create", &kept];
    o.cfg.tools.run(&o.cfg.repo, &create).unwrap();
}

/// Whether the world has started agent `name`.
fn started(w: &World, name: &str) -> bool {
    !w.called(&format!("herdr agent start {name} ")).is_empty()
}

/// PR sessions and the Pipeline count apart: with one slot each, a running
/// Rebase holds no Ticket back, and a running Ticket no Rebase.
#[test]
fn pr_sessions_and_pipeline_tickets_count_apart() {
    for rebase_first in [true, false] {
        let (w, o) = new_world(vec![BdTicket::new("hx-1"), BdTicket::new("hx-2")]);
        set_count(&w.repo, &MAX_TICKETS, Some(1)).unwrap();
        set_count(&w.repo, &MAX_PR_SESSIONS, Some(1)).unwrap();
        set_switch(&w.repo, &REBASE_AUTO, true).unwrap();
        w.session(|p| match p.stage.as_str() {
            "rebase" | "implement" => working(p),
            _ => succeed(p),
        });
        let first = match rebase_first {
            true => {
                pr_open(&w, &o, "hx-1", CONFLICTING);
                w.lock().tickets[1].status = "deferred".to_string(); // not ready yet
                "h-hx-1-rebase"
            }
            false => {
                pr_open(&w, &o, "hx-1", MERGEABLE);
                "h-hx-2-implement"
            }
        };
        let o = Arc::new(o);
        let mut run = spawn_epic(o.clone(), "hx");
        wait_until(first, || started(&w, first));
        let second = match rebase_first {
            true => {
                w.lock().tickets[1].status = "open".to_string();
                "h-hx-2-implement"
            }
            false => {
                let url = "https://example.test/pr/hx-1".to_string();
                w.lock().prs.insert(url, CONFLICTING.to_string());
                "h-hx-1-rebase"
            }
        };
        wait_until(second, || started(&w, second));
        o.stop();
        run.wait();
        o.wait_in_flight();
    }
}

/// Holds `ticket`'s Rebase session inside its prompt until the gate opens.
fn gated(w: &World, ticket: &'static str) -> Arc<AtomicBool> {
    let gate = Arc::new(AtomicBool::new(false));
    let open = gate.clone();
    w.session(move |p| {
        while p.ticket == ticket && p.stage == "rebase" && !open.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(1));
        }
        succeed(p)
    });
    gate
}

/// Past max_pr_sessions a PR session waits, said once, and starts as a
/// slot frees.
#[test]
fn past_max_pr_sessions_a_rebase_waits_for_a_slot() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1"), BdTicket::new("hx-2")]);
    set_count(&w.repo, &MAX_PR_SESSIONS, Some(1)).unwrap();
    set_switch(&w.repo, &REBASE_AUTO, true).unwrap();
    pr_open(&w, &o, "hx-1", CONFLICTING);
    pr_open(&w, &o, "hx-2", CONFLICTING);
    let gate = gated(&w, "hx-1");
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");

    w.await_line("hx-2 rebase waits for a slot");
    thread::sleep(Duration::from_millis(30)); // many more passes
    assert!(started(&w, "h-hx-1-rebase"));
    assert!(!started(&w, "h-hx-2-rebase"), "hx-2 took a second slot");
    gate.store(true, Ordering::SeqCst);
    w.await_line("hx-2 rebased PR #hx-2");
    o.stop();
    run.wait();
    o.wait_in_flight();

    let waits = w
        .lines()
        .iter()
        .filter(|l| l.contains("waits for a slot"))
        .count();
    assert_eq!(waits, 1, "{:#?}", w.lines());
}

/// A Ticket has one PR session at a time: Address PR comments sent while
/// its Rebase runs waits for it.
#[test]
fn address_pr_comments_queues_behind_the_tickets_running_rebase() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    pr_open(&w, &o, "hx-1", CONFLICTING);
    let gate = gated(&w, "hx-1");
    o.command("rebase-hx-1");
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");
    wait_until("the Rebase", || started(&w, "h-hx-1-rebase"));

    o.command("address-pr-comments-hx-1");
    w.await_line("hx-1 address pr comments waits for a slot");
    assert!(!started(&w, "h-hx-1-address-pr-comments"));
    gate.store(true, Ordering::SeqCst);
    w.await_line("hx-1 rebased PR #hx-1");
    w.await_line("hx-1 addressed PR #hx-1");
    o.stop();
    run.wait();
    o.wait_in_flight();
}

/// With rebase_auto on, a PR the poll sees conflict gets a Rebase with no
/// command, once per conflict: a failed one is not restarted until the PR
/// is seen mergeable, then conflicting again.
#[test]
fn rebase_auto_rebases_a_conflicting_pr_once_per_conflict() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    set_switch(&w.repo, &REBASE_AUTO, true).unwrap();
    let url = "https://example.test/pr/hx-1".to_string();
    w.lock().prs.insert(url.clone(), CONFLICTING.to_string());
    let failed = AtomicBool::new(false);
    w.session(
        move |p| match p.stage == "rebase" && !failed.swap(true, Ordering::SeqCst) {
            true => (
                "STATUS: failed\nboth sides rename it\n".to_string(),
                "idle".to_string(),
            ),
            false => succeed(p),
        },
    );
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");

    w.await_line("hx-1 PR #hx-1 conflicts with main, rebasing it");
    w.await_line("hx-1 stuck in rebase");
    o.command("park-hx-1");
    w.await_line("hx-1 rebase gave up");
    thread::sleep(Duration::from_millis(30)); // many more polls
    assert_eq!(w.called("herdr agent start h-hx-1-rebase ").len(), 1);

    w.lock().prs.insert(url.clone(), MERGEABLE.to_string());
    wait_until("the conflict cleared", || !o.ticket("hx-1").conflict);
    w.lock().prs.insert(url, CONFLICTING.to_string());
    w.await_line("hx-1 rebased PR #hx-1");
    o.stop();
    run.wait();
    o.wait_in_flight();
    assert_eq!(w.called("herdr agent start h-hx-1-rebase ").len(), 2);
}

/// A Rebase that waited for a slot while its PR stopped conflicting is
/// refused, and the next conflict queues one again.
#[test]
fn a_rebase_refused_after_its_wait_comes_back_with_the_next_conflict() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1"), BdTicket::new("hx-2")]);
    set_count(&w.repo, &MAX_PR_SESSIONS, Some(1)).unwrap();
    set_switch(&w.repo, &REBASE_AUTO, true).unwrap();
    pr_open(&w, &o, "hx-1", CONFLICTING);
    pr_open(&w, &o, "hx-2", CONFLICTING);
    let gate = gated(&w, "hx-1");
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");
    w.await_line("hx-2 rebase waits for a slot");

    // GitHub works the merge state out again after main moves
    let url = "https://example.test/pr/hx-2".to_string();
    let unknown = r#"{"state":"OPEN","mergeable":"UNKNOWN"}"#;
    w.lock().prs.insert(url.clone(), unknown.to_string());
    wait_until("UNKNOWN polled", || !o.ticket("hx-2").conflicting);
    gate.store(true, Ordering::SeqCst);
    w.await_line("hx-2 refused: PR #hx-2 does not conflict with main");
    w.lock().prs.insert(url, CONFLICTING.to_string());
    w.await_line("hx-2 rebased PR #hx-2");
    o.stop();
    run.wait();
    o.wait_in_flight();
}

/// Away, a Rebase's question parks its Ticket, a bd comment and no
/// Question, and the next PR session in the queue takes its slot. Its PR
/// turning CONFLICTING again starts nothing. /continue @ticket puts the
/// question of the session still waiting in its pane, whose answer goes
/// there; once the Rebase is done the Ticket is pr-open, Implement never
/// started.
#[test]
fn away_parks_a_rebase_question_frees_its_slot_and_continue_puts_it() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1"), BdTicket::new("hx-2")]);
    set_count(&w.repo, &MAX_PR_SESSIONS, Some(1)).unwrap();
    set_switch(&w.repo, &REBASE_AUTO, true).unwrap();
    o.cfg.away.store(true, Ordering::SeqCst);
    pr_open(&w, &o, "hx-1", CONFLICTING);
    pr_open(&w, &o, "hx-2", CONFLICTING);
    w.session(
        |p| match (p.ticket.as_str(), p.stage.as_str(), p.text.as_str()) {
            (_, _, "ours") => (String::new(), "working".to_string()),
            ("hx-1", "rebase", _) => (ASKS.to_string(), "idle".to_string()),
            _ => succeed(p),
        },
    );
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");

    w.await_line("hx-1 parked: asked you while away");
    w.await_line("hx-2 rebased PR #hx-2");
    let ts = o.ticket("hx-1");
    assert_eq!(
        (ts.status.as_str(), ts.stage.as_str(), ts.reason.as_str()),
        (STATUS_PARKED, "rebase", AWAY)
    );
    assert_eq!(w.called("bd comments add hx-1 ").len(), 1);
    assert!(
        w.events().iter().all(|e| e.ask.is_none()),
        "Away put a Question"
    );

    let url = "https://example.test/pr/hx-1".to_string();
    w.lock().prs.insert(url.clone(), MERGEABLE.to_string());
    thread::sleep(Duration::from_millis(30)); // many more polls
    w.lock().prs.insert(url, CONFLICTING.to_string());
    thread::sleep(Duration::from_millis(30));
    let said = w.lines();
    let conflicts = said
        .iter()
        .filter(|l| l.contains("hx-1 PR #hx-1 conflicts"))
        .count();
    assert_eq!(conflicts, 1, "{said:#?}");
    assert_eq!(w.called("herdr agent start h-hx-1-rebase ").len(), 1);

    o.cfg.away.store(false, Ordering::SeqCst); // /continue turns Away off
    o.command("continue-hx-1");
    let asked = w.await_event("question in rebase");
    let Some(Ask::StageQuestion { pane, question, .. }) = asked.ask else {
        panic!("no Question: {:?}", asked.ask);
    };
    assert_eq!(question, "Which parser stays?");
    o.answer("hx-1", &pane, Answer::Prompt("ours".to_string()));
    w.await_line("hx-1 sent your answer");
    write_file(&o.run_dir("hx-1").join("rebase.md"), "STATUS: done\n");
    w.lock().agents.insert(pane.clone(), "idle".to_string());
    w.await_line("hx-1 rebased PR #hx-1");
    o.stop();
    run.wait();
    o.wait_in_flight();

    assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN);
    assert_eq!(
        w.called(&format!("herdr agent prompt {pane} ours")).len(),
        1
    );
    assert_eq!(
        w.called("herdr agent start h-hx-1-rebase ").len(),
        1,
        "not watched"
    );
    assert!(
        !started(&w, "h-hx-1-implement"),
        "/continue went to the Pipeline"
    );
}

/// A PR merged while its Rebase runs stays merged when the Rebase then
/// asks while Away: not Parked, not polled, not closed twice.
#[test]
fn a_pr_merged_while_its_rebase_runs_stays_merged_when_it_asks() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    o.cfg.away.store(true, Ordering::SeqCst);
    pr_open(&w, &o, "hx-1", CONFLICTING);
    let gate = Arc::new(AtomicBool::new(false));
    let open = gate.clone();
    w.session(move |p| {
        while p.stage == "rebase" && !open.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(1));
        }
        (ASKS.to_string(), "idle".to_string())
    });
    o.command("rebase-hx-1");
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");
    wait_until("the Rebase", || started(&w, "h-hx-1-rebase"));

    let merged = r#"{"state":"MERGED","mergeable":"UNKNOWN"}"#.to_string();
    w.lock()
        .prs
        .insert("https://example.test/pr/hx-1".to_string(), merged);
    w.await_line("hx-1 merged, Ticket closed");
    gate.store(true, Ordering::SeqCst);
    w.await_line("hx-1 rebase gave up: asked you while away");
    thread::sleep(Duration::from_millis(30)); // many more polls
    o.stop();
    run.wait();
    o.wait_in_flight();
    assert_eq!(o.ticket("hx-1").status, STATUS_MERGED);
    assert_eq!(w.called("bd close hx-1 ").len(), 1);
}

/// A PR merged while /continue fetches its comments again, the fetch
/// failing, stays merged: not Parked, not closed twice.
#[test]
fn a_pr_merged_while_a_continued_fetch_fails_stays_merged() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    pr_open(&w, &o, "hx-1", MERGEABLE);
    o.update("hx-1", |ts| {
        ts.status = STATUS_PARKED.to_string();
        ts.stage = "address-pr-comments".to_string();
        ts.reason = AWAY.to_string();
    });
    let o = Arc::new(o);
    let (world, orq) = (Arc::downgrade(&w), Arc::downgrade(&o));
    w.hook(move |_, argv| {
        if argv.contains(&"reviews,comments,statusCheckRollup") {
            let merged = r#"{"state":"MERGED","mergeable":"UNKNOWN"}"#.to_string();
            let w = world.upgrade()?;
            w.lock()
                .prs
                .insert("https://example.test/pr/hx-1".to_string(), merged);
            let o = orq.upgrade()?;
            wait_until("the merge", || o.ticket("hx-1").status == STATUS_MERGED);
            return Some(Err("gh: boom".to_string()));
        }
        None
    });
    o.command("continue-hx-1");
    let mut run = spawn_epic(o.clone(), "hx");
    w.await_line("hx-1 address pr comments failed: ");
    thread::sleep(Duration::from_millis(30)); // many more polls
    o.stop();
    run.wait();
    o.wait_in_flight();
    assert_eq!(o.ticket("hx-1").status, STATUS_MERGED);
    assert_eq!(w.called("bd close hx-1 ").len(), 1);
    let said = w.lines();
    assert!(!said.iter().any(|l| l.contains("parked")), "{said:#?}");
}

/// A PR the poll sees merged while /address-pr-comments fetches its
/// comments stays merged: no session starts in the removed worktree.
#[test]
fn a_pr_merged_while_its_comments_are_fetched_starts_no_session() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    pr_open(&w, &o, "hx-1", MERGEABLE);
    let o = Arc::new(o);
    let (world, orq) = (Arc::downgrade(&w), Arc::downgrade(&o));
    w.hook(move |_, argv| {
        if argv.contains(&"reviews,comments,statusCheckRollup") {
            let merged = r#"{"state":"MERGED","mergeable":"UNKNOWN"}"#.to_string();
            let w = world.upgrade()?;
            w.lock()
                .prs
                .insert("https://example.test/pr/hx-1".to_string(), merged);
            let o = orq.upgrade()?;
            wait_until("the merge", || o.ticket("hx-1").status == STATUS_MERGED);
        }
        None
    });
    o.command("address-pr-comments-hx-1");
    let mut run = spawn_epic(o.clone(), "hx");
    w.await_line("hx-1 merged, Ticket closed");
    thread::sleep(Duration::from_millis(30)); // many more polls
    o.stop();
    run.wait();
    o.wait_in_flight();
    assert!(!started(&w, "h-hx-1-address-pr-comments"));
    assert_eq!(o.ticket("hx-1").status, STATUS_MERGED);
    assert_eq!(w.called("bd close hx-1 ").len(), 1);
}
