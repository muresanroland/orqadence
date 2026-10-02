//! The approval modal over the world: a quiet head's new PR comments put to
//! the user, its countdown on the fake clock, Away, the switch, the runs cap
//! and /address-pr-comments.

use chrono::{DateTime, Local, TimeDelta, TimeZone};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::brand::{GREEN, PURPLE, RED};
use super::shell_test::{
    await_end, await_line, await_questions, find, key, line, pick, render, row, rows, screen_at,
    shell,
};
use super::{Approval, NoticeKind, Screen};
use crate::orchestrator::app::{set_switch, ADDRESS_PR_COMMENTS_AUTO, AGENT_MERGE};
use crate::orchestrator::pr::Item;
use crate::orchestrator::stage::Orchestrator;
use crate::orchestrator::state::STATUS_PR_OPEN;
use crate::orchestrator::world::{new_world, set_clock, wait_until, BdTicket, World};
use crate::tools::fake::Fake;

/// An open PR on head `head`: a failing check run, then CodeRabbit's open
/// thread rated Major, as the poll's GraphQL query shapes it.
fn commented(head: &str) -> Value {
    json!({
        "state": "OPEN",
        "mergeable": "MERGEABLE",
        "headRefOid": head,
        "statusCheckRollup": {"commit": {"oid": head}, "contexts": {"nodes": [{
            "name": "test", "status": "COMPLETED", "conclusion": "FAILURE",
            "startedAt": "2026-09-30T12:00:00Z",
            "checkSuite": {"app": {"slug": "github-actions"},
                "workflowRun": {"event": "pull_request", "workflow": {"name": "CI"}}},
        }]}},
        "reviewThreads": {"nodes": [{
            "id": "T1", "isResolved": false, "path": "src/a.rs", "line": 3,
            "comments": {"nodes": [{
                "databaseId": 1,
                "author": {"login": "coderabbitai", "__typename": "Bot"},
                "body": "_⚠️ Potential issue_ | _🟠 Major_\n\n**Guard the empty list**",
            }]},
        }]},
        "reviews": {"nodes": []},
        "comments": {"nodes": []},
    })
}

fn url(ticket: &str) -> String {
    format!("https://example.test/pr/{ticket}")
}

fn serve(w: &World, ticket: &str, pr: &Value) {
    w.lock().prs.insert(url(ticket), pr.to_string());
}

/// The live run's Orchestrator.
fn run(s: &Screen) -> Arc<Orchestrator> {
    s.run.as_ref().expect("no run is live").o.clone()
}

/// A run continued over `tickets`, each with its PR open as `pr` and its
/// worktree kept, address_pr_comments_auto as `auto`, the clock at noon;
/// back once the poll has seen every PR's head.
fn polled(
    tickets: &[&str],
    pr: &Value,
    auto: bool,
) -> (Arc<World>, Screen, Arc<Mutex<DateTime<Local>>>) {
    let (w, o) = new_world(tickets.iter().map(|t| BdTicket::new(t)).collect());
    set_switch(&w.repo, &ADDRESS_PR_COMMENTS_AUTO, auto).unwrap();
    for &ticket in tickets {
        serve(&w, ticket, pr);
        o.update(ticket, |ts| {
            ts.status = STATUS_PR_OPEN.to_string();
            ts.pr = url(ticket);
        });
        let kept = o.worktree(ticket).display().to_string();
        let create = ["bd", "worktree", "create", &kept];
        o.cfg.tools.run(&o.cfg.repo, &create).unwrap();
    }
    let mut s = shell(&w);
    let noon = Local.with_ymd_and_hms(2026, 9, 30, 12, 0, 0).unwrap();
    let clock = set_clock(&mut s.cfg, noon);
    s.command("/continue");
    let o = run(&s);
    let head = pr["headRefOid"].as_str().unwrap();
    wait_until("the poll seeing each head", || {
        tickets.iter().all(|t| o.ticket(t).head == head)
    });
    (w, s, clock)
}

fn later(clock: &Mutex<DateTime<Local>>, seconds: i64) {
    *clock.lock().unwrap() += TimeDelta::seconds(seconds);
}

/// Polls the Shell until `n` approval modals wait.
fn await_approvals(s: &mut Screen, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while s.approvals.len() != n {
        assert!(Instant::now() < deadline, "{n} modals never waited");
        s.poll();
        thread::sleep(Duration::from_millis(1));
    }
}

/// Each row's summary and whether it is checked.
fn checked(a: &Approval) -> Vec<(&str, bool)> {
    a.rows
        .iter()
        .map(|(item, on)| (item.summary.as_str(), *on))
        .collect()
}

/// A quiet head's new items open the modal, not a Question: one row each,
/// most severe first, all checked. Space unchecks a row, and Enter starts
/// Address PR comments with the checked rows approved, the others won't
/// fix, and the PR's checks, counting the run.
#[test]
fn a_quiet_head_opens_the_modal_and_enter_starts_address_pr_comments_with_the_lists() {
    let (w, mut s, clock) = polled(&["hx-1"], &commented("a"), true);
    later(&clock, 60);
    await_approvals(&mut s, 1);
    assert!(
        s.questions.is_empty(),
        "the modal joined the Question queue"
    );
    assert_eq!(s.approvals[0].ticket, "hx-1");
    let all = [("test", true), ("Guard the empty list", true)];
    assert_eq!(checked(&s.approvals[0]), all);

    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Char(' ')));
    let some = [("test", true), ("Guard the empty list", false)];
    assert_eq!(checked(&s.approvals[0]), some);
    s.key(key(KeyCode::Enter));
    assert!(s.approvals.is_empty());
    await_line(&mut s, "hx-1 you approved 1 of 2 PR comments");
    await_line(&mut s, "hx-1 addressed PR #hx-1");

    let prompt = w.prompt("address-pr-comments.md");
    let lists = "- Approved: \n  - check by github-actions — test\n\
                 - Won't fix: \n  - thread by coderabbitai at src/a.rs:3 — Guard the empty list\n";
    assert!(prompt.contains(lists), "{prompt}");
    assert!(prompt.contains(r#""conclusion":"FAILURE""#), "{prompt}");
    assert_eq!(run(&s).ticket("hx-1").address_runs, 1);
}

/// Waits for two more polls of gh: one at least ran wholly after this call.
fn await_polls(w: &World) {
    let before = w.called("gh api graphql").len();
    wait_until("two more polls", || {
        w.called("gh api graphql").len() >= before + 2
    });
}

/// Whether Address PR comments has started for `ticket`.
fn addressing(w: &World, ticket: &str) -> bool {
    let start = format!("herdr agent start h-{ticket}-address-pr-comments ");
    !w.called(&start).is_empty()
}

/// Cancel starts nothing and the poll does not reopen what it offered;
/// /address-pr-comments opens it again by hand with every item still open,
/// and no countdown.
#[test]
fn esc_starts_nothing_and_only_address_pr_comments_reopens_it() {
    let (w, mut s, clock) = polled(&["hx-1"], &commented("a"), true);
    later(&clock, 60);
    await_approvals(&mut s, 1);
    s.key(key(KeyCode::Esc));
    assert!(s.approvals.is_empty());
    await_line(
        &mut s,
        "hx-1 you cancelled 2 PR comments, /address-pr-comments opens them",
    );
    await_polls(&w);
    s.poll();
    assert!(s.approvals.is_empty(), "the poll reopened it");
    assert!(!addressing(&w, "hx-1"));

    // the thread answered since: only the check is still open
    let mut pr = commented("a");
    pr["reviewThreads"]["nodes"][0]["isResolved"] = json!(true);
    serve(&w, "hx-1", &pr);
    s.command("/address-pr-comments @hx-1");
    assert_eq!(s.approvals.len(), 1, "{:?}", s.notice);
    assert_eq!(checked(&s.approvals[0]), [("test", true)]);
    assert_eq!(s.approvals[0].approves, None, "a countdown by hand");
}

/// Under Agent merge the merge Question waits for the modal: while it is up
/// Address PR comments' flow is not over. Cancelled, the Question comes, and
/// merge answered there merges the PR with its PR comment open.
#[test]
fn a_cancelled_modal_is_followed_by_the_merge_question_and_merge_merges() {
    let mut pr = commented("a");
    pr["statusCheckRollup"]["contexts"]["nodes"] = json!([]); // green
    let (w, mut s, clock) = polled(&["hx-1"], &pr, true);
    set_switch(&w.repo, &AGENT_MERGE, true).unwrap();
    later(&clock, 60);
    await_approvals(&mut s, 1);
    await_polls(&w);
    s.poll();
    assert!(s.questions.is_empty(), "asked while its modal waits");

    s.key(key(KeyCode::Esc));
    await_questions(&mut s, 1);
    assert_eq!(
        s.questions[0].text,
        "PR #hx-1: 1 PR comment open, merge it?"
    );
    assert_eq!(s.options(), ["merge", "park"]);
    s.key(key(KeyCode::Esc)); // the Epic summary, open since every Ticket has its PR
    pick(&mut s, 1);
    await_line(&mut s, "hx-1 PR #hx-1 merged by Orqadence");
    assert_eq!(w.called("gh pr merge").len(), 1);
}

/// A line of its Ticket closes the merge Question, as it closes any; the
/// Orchestrator, told so, asks it again.
#[test]
fn a_merge_question_a_ticket_line_closed_is_asked_again() {
    let mut pr = commented("a");
    pr["statusCheckRollup"]["contexts"]["nodes"] = json!([]); // green
    let (w, mut s, clock) = polled(&["hx-1"], &pr, true);
    set_switch(&w.repo, &AGENT_MERGE, true).unwrap();
    later(&clock, 60);
    await_approvals(&mut s, 1);
    s.key(key(KeyCode::Esc));
    await_questions(&mut s, 1);

    run(&s).report("hx-1", "a line of its own");
    await_line(&mut s, "hx-1 a line of its own");
    let asking = "hx-1 asking you: PR #hx-1: 1 PR comment open, merge it?";
    let asked = |s: &Screen| s.events.iter().filter(|e| line(e) == asking).count();
    let deadline = Instant::now() + Duration::from_secs(5);
    while asked(&s) < 2 {
        assert!(Instant::now() < deadline, "never asked again");
        s.poll();
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(s.questions.len(), 1);
}

/// The countdown, address_pr_comments_countdown's 5 minutes from when the
/// modal shows, approves the rows as they stand at zero. After the push the
/// bots' new items open a new modal, and a key pressed in it stops its
/// countdown for good: nothing starts by itself.
#[test]
fn the_countdown_approves_at_zero_until_a_key_stops_it() {
    let (w, mut s, clock) = polled(&["hx-1"], &commented("a"), true);
    later(&clock, 60);
    await_approvals(&mut s, 1);
    later(&clock, 299);
    s.tick();
    assert_eq!(s.approvals.len(), 1, "approved early");
    later(&clock, 1);
    s.tick();
    assert!(s.approvals.is_empty(), "never approved itself");
    await_line(&mut s, "hx-1 the countdown approved 2 of 2 PR comments");
    await_line(&mut s, "hx-1 addressed PR #hx-1");

    // the push: head b, its check failing again and a new thread
    let mut pr = commented("b");
    let threads = pr["reviewThreads"]["nodes"].as_array_mut().unwrap();
    let mut minor = threads[0].clone();
    minor["id"] = json!("T2");
    minor["comments"]["nodes"][0]["body"] = json!("_🟡 Minor_\n\n**Name the magic number**");
    threads.push(minor);
    serve(&w, "hx-1", &pr);
    let o = run(&s);
    wait_until("the poll seeing b", || o.ticket("hx-1").head == "b");
    later(&clock, 60);
    await_approvals(&mut s, 1);
    let new = [("test", true), ("Name the magic number", true)];
    assert_eq!(checked(&s.approvals[0]), new);
    // a first Ctrl-C is a key too
    s.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    later(&clock, 600);
    s.tick();
    assert_eq!(s.approvals.len(), 1, "approved after a key");
    await_polls(&w);
    let starts = w.called("herdr agent start h-hx-1-address-pr-comments ");
    assert_eq!(starts.len(), 1, "a second run started by itself");
}

/// Away: no modal; every item is approved as offered, a RECENT line says
/// so, and Address PR comments starts with nothing won't fix.
#[test]
fn away_approves_every_item_without_a_modal() {
    let (w, mut s, clock) = polled(&["hx-1"], &commented("a"), true);
    s.command("/away");
    later(&clock, 60);
    await_line(&mut s, "hx-1 PR #hx-1: 2 PR comments approved while away");
    await_line(&mut s, "hx-1 addressed PR #hx-1");
    assert!(s.approvals.is_empty());
    let lists = "- Approved: \n  - check by github-actions — test\n  \
                 - thread by coderabbitai at src/a.rs:3 — Guard the empty list\n\
                 - Won't fix: none\n";
    let prompt = w.prompt("address-pr-comments.md");
    assert!(prompt.contains(lists), "{prompt}");
}

/// With address_pr_comments_auto off the poll only says so, once for the
/// set of items.
#[test]
fn with_the_switch_off_only_the_line_appears_once() {
    let (w, mut s, clock) = polled(&["hx-1"], &commented("a"), false);
    later(&clock, 60);
    let said = "hx-1 PR #hx-1: 2 PR comments, /address-pr-comments opens them";
    await_line(&mut s, said);
    await_polls(&w);
    s.poll();
    let lines = s.events.iter().filter(|e| line(e) == said).count();
    assert_eq!(lines, 1);
    assert!(s.approvals.is_empty());
    assert!(!addressing(&w, "hx-1"));
}

/// Past address_pr_comments_runs runs new items only get a line naming the
/// cap; /address-pr-comments still opens the modal.
#[test]
fn past_the_runs_cap_new_items_get_a_line_and_the_command_still_opens_them() {
    let (w, mut s, clock) = polled(&["hx-1"], &commented("a"), true);
    run(&s).update("hx-1", |ts| ts.address_runs = 3);
    later(&clock, 60);
    await_line(
        &mut s,
        "hx-1 PR #hx-1: 2 PR comments, past the cap of 3 Address PR comments runs, \
         /address-pr-comments opens them",
    );
    await_polls(&w);
    s.poll();
    assert!(s.approvals.is_empty());
    s.command("/address-pr-comments hx-1");
    assert_eq!(s.approvals.len(), 1, "{:?}", s.notice);
}

/// The cap holds when the run starts, not only when the modal is offered:
/// a modal approved after another run reached the cap starts nothing. One
/// opened by hand still runs.
#[test]
fn a_modal_approved_past_the_runs_cap_starts_nothing_but_one_by_hand_runs() {
    let (w, mut s, clock) = polled(&["hx-1"], &commented("a"), true);
    run(&s).update("hx-1", |ts| ts.address_runs = 2);
    later(&clock, 60);
    await_approvals(&mut s, 1);
    run(&s).update("hx-1", |ts| ts.address_runs = 3);
    s.key(key(KeyCode::Enter));
    await_line(
        &mut s,
        "hx-1 PR #hx-1: past the cap of 3 Address PR comments runs, \
         /address-pr-comments opens them",
    );
    assert!(!addressing(&w, "hx-1"));
    s.command("/address-pr-comments hx-1");
    s.key(key(KeyCode::Enter));
    await_line(&mut s, "hx-1 addressed PR #hx-1");
    assert_eq!(run(&s).ticket("hx-1").address_runs, 4);
}

/// gh failing as Address PR comments starts takes its approved items out
/// of offered, so the poll opens them again.
#[test]
fn a_gh_failure_at_the_start_offers_the_items_again() {
    let (w, mut s, clock) = polled(&["hx-1"], &commented("a"), true);
    later(&clock, 60);
    await_approvals(&mut s, 1);
    w.fail_once("gh pr view", "gh: boom");
    s.key(key(KeyCode::Enter));
    await_line(
        &mut s,
        "hx-1 address pr comments failed: gh pr view https://example.test/pr/hx-1 \
         --json reviews,comments,statusCheckRollup: exit status 1: gh: boom",
    );
    await_approvals(&mut s, 1);
    assert!(!addressing(&w, "hx-1"));
}

/// Exiting with a modal unanswered, by Ctrl-C twice as /exit does, withdraws
/// its items from offered, so the next run's poll opens them again.
#[test]
fn a_modal_left_at_exit_is_withdrawn() {
    let (_w, mut s, clock) = polled(&["hx-1"], &commented("a"), true);
    later(&clock, 60);
    await_approvals(&mut s, 1);
    let o = run(&s);
    assert!(!o.ticket("hx-1").offered.is_empty());
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    s.key(ctrl_c);
    s.key(ctrl_c);
    assert!(s.quit);
    s.close();
    assert!(s.approvals.is_empty());
    assert!(o.ticket("hx-1").offered.is_empty());
}

/// An offer the Shell has not read yet at exit, its items already saved
/// as offered, is withdrawn too.
#[test]
fn an_offer_unread_at_exit_is_withdrawn() {
    let (_w, mut s, clock) = polled(&["hx-1"], &commented("a"), true);
    let o = run(&s);
    later(&clock, 60);
    wait_until("the poll offering", || !o.ticket("hx-1").offered.is_empty());
    s.close();
    assert!(s.approvals.is_empty());
    assert!(o.ticket("hx-1").offered.is_empty());
}

/// An offer sent after the poll last read the Events, as the run stops,
/// is withdrawn with the run: /continue offers its items again.
#[test]
fn an_offer_unread_when_the_run_ends_is_withdrawn() {
    let (_w, mut s, clock) = polled(&["hx-1"], &commented("a"), true);
    let o = run(&s);
    later(&clock, 60);
    wait_until("the poll offering", || !o.ticket("hx-1").offered.is_empty());
    let mut ended = s.run.take().unwrap();
    ended.o.stop();
    let _ = ended.scheduler.take().unwrap().join();
    s.withdraw(&ended);
    assert!(s.approvals.is_empty(), "an orphan modal opened");
    assert!(o.ticket("hx-1").offered.is_empty());
}

/// A Notice modal over the approval modal takes its keys, so its countdown
/// holds; once the Notice closes it runs again in full.
#[test]
fn a_notice_over_the_modal_holds_its_countdown() {
    let (_w, mut s, clock) = polled(&["hx-1"], &commented("a"), true);
    later(&clock, 60);
    await_approvals(&mut s, 1);
    s.notify(NoticeKind::Info, "news", None);
    later(&clock, 300);
    s.tick();
    assert_eq!(s.approvals.len(), 1, "approved under the Notice");
    s.key(key(KeyCode::Enter));
    assert!(s.notices.is_empty());
    later(&clock, 299);
    s.tick();
    assert_eq!(s.approvals.len(), 1, "the countdown did not start again");
    later(&clock, 1);
    s.tick();
    assert!(s.approvals.is_empty(), "never approved itself");
}

/// Two PRs with new items: one modal shows, the other waits its turn, its
/// countdown not running until it shows.
#[test]
fn two_prs_show_one_modal_then_the_other() {
    let (_w, mut s, clock) = polled(&["hx-1", "hx-2"], &commented("a"), true);
    later(&clock, 60);
    await_approvals(&mut s, 2);
    let shows = |s: &Screen| {
        (
            s.approvals[0].ticket.clone(),
            s.approvals[0].approves.is_some(),
        )
    };
    assert_eq!(shows(&s), ("hx-1".to_string(), true));
    assert_eq!(
        s.approvals[1].approves, None,
        "its countdown ran while it waited"
    );
    s.key(key(KeyCode::Esc));
    assert_eq!(shows(&s), ("hx-2".to_string(), true));
}

/// Rebase goes first: a PR that conflicts with main gets no modal.
#[test]
fn a_conflicting_pr_gets_no_modal() {
    let mut pr = commented("a");
    pr["mergeable"] = json!("CONFLICTING");
    let (w, mut s, clock) = polled(&["hx-1"], &pr, true);
    await_line(
        &mut s,
        "hx-1 PR #hx-1 conflicts with main, /rebase resolves it",
    );
    later(&clock, 60);
    await_polls(&w);
    s.poll();
    assert!(s.approvals.is_empty());
}

fn item(id: &str, kind: &'static str, summary: &str, rating: &str, place: &str) -> Item {
    Item {
        id: id.to_string(),
        kind,
        author: "coderabbitai".to_string(),
        summary: summary.to_string(),
        rating: rating.to_string(),
        place: place.to_string(),
    }
}

/// The idle Shell's Ticket 9 with two PRs' modals waiting, the clock at
/// noon: 9's shows.
fn modal_screen() -> Screen {
    let mut s = screen_at(Fake::quiet(), Path::new(""));
    let noon = Local.with_ymd_and_hms(2026, 9, 30, 12, 0, 0).unwrap();
    set_clock(&mut s.cfg, noon);
    let pr = "https://github.com/o/r/pull/12".to_string();
    s.state.tickets.get_mut("harness-kqe.9").unwrap().pr = pr;
    let items = vec![
        item("c", "check", "test", "failed", ""),
        item("t", "thread", "Guard the empty list", "Major", "src/a.rs:3"),
    ];
    let five = Some(Duration::from_secs(300));
    s.offer("harness-kqe.9".to_string(), items.clone(), five, false);
    s.offer("harness-kqe.10".to_string(), items, five, false);
    s
}

/// From 110 columns the modal docks beside the Shell in the right 58%: the
/// Ticket and its PR, the other PR waiting, a row per item with its
/// checkbox, summary and rating, [ Fix comments ] green and [ Cancel ] red,
/// and the countdown at its foot. Under, it folds over the dimmed Shell.
#[test]
fn the_modal_docks_at_140_columns_and_folds_at_90() {
    let mut s = modal_screen();
    let buf = render(&s, 140, 40);
    let (x, y) = find(&buf, " PR COMMENTS · 9 The Shell, idle").unwrap();
    assert!(
        x > 58 && y == 0,
        "not docked right: {x},{y}\n{:#?}",
        rows(&buf)
    );
    assert!(row(&buf, 1).contains("PR #12 · 2 PR comments · 1 more PR waiting"));
    let (x, y) = find(&buf, "› [x] test").unwrap();
    assert!(
        row(&buf, y).trim_end().ends_with("failed ┃"),
        "{:?}",
        row(&buf, y)
    );
    assert_eq!(buf[(x, y)].fg, PURPLE, "the cursor's row is not marked");
    let thread = row(&buf, y + 1);
    assert!(thread.contains("  [x] Guard the empty list"), "{thread:?}");
    assert!(thread.trim_end().ends_with("Major ┃"), "{thread:?}");
    let (x, y) = find(&buf, "[ Fix comments ]").unwrap();
    assert_eq!(buf[(x, y)].bg, GREEN);
    let (x, y) = find(&buf, "[ Cancel ]").unwrap();
    assert_eq!(buf[(x, y)].bg, RED);
    assert!(
        find(&buf, "approves in 5:00").is_some(),
        "{:#?}",
        rows(&buf)
    );

    s.key(key(KeyCode::Char(' ')));
    let buf = render(&s, 90, 30);
    assert_eq!(find(&buf, "╭"), Some((0, 0)), "{:#?}", rows(&buf));
    assert!(row(&buf, 0).contains(" PR COMMENTS · 9 The Shell, idle"));
    assert!(find(&buf, "› [ ] test").is_some(), "{:#?}", rows(&buf));
    assert!(find(&buf, "[ Fix comments ]").is_some());
    assert!(find(&buf, "approves in").is_none(), "a key did not stop it");
    assert!(
        row(&buf, 29).starts_with("› "),
        "the input line went under the fold"
    );
}

/// A modal left unanswered when the run stops is not lost: the poll of the
/// run /continue resumes opens it again.
#[test]
fn a_modal_left_at_stop_work_opens_again_on_continue() {
    let (_w, mut s, clock) = polled(&["hx-1"], &commented("a"), true);
    later(&clock, 60);
    await_approvals(&mut s, 1);
    s.command("/stop-work");
    await_end(&mut s);
    assert!(s.approvals.is_empty());
    s.command("/continue");
    await_approvals(&mut s, 1);
    let all = [("test", true), ("Guard the empty list", true)];
    assert_eq!(checked(&s.approvals[0]), all);
}
