use super::app::set_max_tickets;
use super::stage::{Config, Orchestrator};
use super::state::{
    acquire_lock, load_state, lock_holder, STATUS_MERGED, STATUS_PARKED, STATUS_PR_OPEN,
    STATUS_RUNNING,
};
use super::world::{new_world, spawn_epic, succeed, BdTicket, Prompt, World};
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
        set_max_tickets(&w.repo, Some(max)).unwrap();
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

    w.await_line("hx-2 waiting for PR #hx-1 to merge (Ticket hx-1)");
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

/// /rebase on a PR that does not conflict is refused, and no Stage starts.
#[test]
fn rebase_command_on_a_mergeable_pr_is_refused() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");
    w.await_line("hx-1 PR #hx-1 opened");

    o.command("rebase-hx-1");
    w.await_line("hx-1 refused: PR #hx-1 does not conflict with main");
    o.stop();
    run.wait();
    o.wait_in_flight();
    assert!(w.called("herdr agent start h-hx-1-rebase").is_empty());
}

/// /address-pr-comments: a fresh session fed the PR and gh's view of its
/// comments; the Ticket stays pr-open.
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
        "- PR comments (gh JSON): ",
        "rename this",
    ] {
        assert!(
            text.contains(want),
            "address-pr-comments prompt lacks {want:?}:\n{text}"
        );
    }
    assert_eq!(
        o.ticket("hx-1").status,
        STATUS_PR_OPEN,
        "status after address-pr-comments, want it still pr-open"
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
        if argv.join(" ").contains("reviews") && gh_down.swap(false, Ordering::SeqCst) {
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
