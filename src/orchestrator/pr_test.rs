//! The PR poll's reply: its items over trimmed real replies of this repo's
//! PRs, and the quiet head on the world's clock.

use chrono::{DateTime, Local, TimeDelta, TimeZone};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use super::app::{set_review_bots, set_switch, ADDRESS_PR_COMMENTS_AUTO, AGENT_MERGE, REBASE_AUTO};
use super::pr::{Item, Pr};
use super::scheduler_test::with_deps;
use super::stage::{Orchestrator, ADDRESS_PR_COMMENTS, AWAY, REBASE};
use super::state::{STATUS_MERGED, STATUS_PARKED, STATUS_PR_OPEN, STATUS_RUNNING};
use super::world::{new_world, set_clock, spawn_epic, BdTicket, World};

/// Greptile: resolved threads answered with the older marker, six
/// outside-diff findings, a summary without a Findings list.
const PR29: &str = include_str!("testdata/pr/29.json");
/// Greptile: four open threads, the summary's Findings linking them, two
/// outside-diff findings, its check run.
const PR34: &str = include_str!("testdata/pr/34.json");
/// CodeRabbit: a nitpick answered by a marker PR comment, beside a 🔇 LGTM
/// entry taken from Chloemlla/LibChecker#15.
const PR37: &str = include_str!("testdata/pr/37.json");
/// CodeRabbit: three resolved threads, each answered with the marker and a
/// bot reply after it.
const PR52: &str = include_str!("testdata/pr/52.json");
/// CodeRabbit: one resolved Minor thread, its status context.
const PR65: &str = include_str!("testdata/pr/65.json");
/// CodeRabbit: two Minor threads asking to delete the Moshi docs that
/// harness-we9.3 builds. Each thread's reply is not on the real PR: it is
/// the answer address-pr-comments gives a comment another Ticket covers.
const PR78: &str = include_str!("testdata/pr/78.json");

fn fixture(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap()
}

fn items(pr: &Value) -> Vec<Item> {
    serde_json::from_value::<Pr>(pr.clone()).unwrap().items()
}

/// kind, rating, place, summary: what a test reads off an item.
fn rows(items: &[Item]) -> Vec<(&str, &str, &str, &str)> {
    items
        .iter()
        .map(|i| {
            (
                i.kind,
                i.rating.as_str(),
                i.place.as_str(),
                i.summary.as_str(),
            )
        })
        .collect()
}

fn thread(pr: &mut Value, i: usize) -> &mut Value {
    &mut pr["reviewThreads"]["nodes"][i]
}

fn comments(value: &mut Value) -> &mut Vec<Value> {
    value["comments"]["nodes"].as_array_mut().unwrap()
}

fn human(id: u64, body: &str) -> Value {
    json!({
        "databaseId": id,
        "url": format!("https://github.com/muresanroland/orqadence/pull/34#issuecomment-{id}"),
        "author": {"login": "muresanroland", "__typename": "User"},
        "body": body,
    })
}

#[test]
fn a_coderabbit_thread_rated_major_is_an_item_rated_major() {
    let mut pr = fixture(PR52);
    thread(&mut pr, 0)["isResolved"] = json!(false);
    comments(thread(&mut pr, 0)).truncate(1); // not answered yet

    let items = items(&pr);
    assert_eq!(
        rows(&items),
        [(
            "thread",
            "Major",
            "src/orchestrator/cost.rs:153",
            "Bound session discovery on the shell thread."
        )]
    );
    assert_eq!(items[0].id, "PRRT_kwDOUiwtFs6mPl8g");
    assert_eq!(items[0].author, "coderabbitai");
}

#[test]
fn a_resolved_thread_is_absent() {
    let mut pr = fixture(PR65);
    assert_eq!(items(&pr), [], "#65's one thread is resolved");

    thread(&mut pr, 0)["isResolved"] = json!(false);
    assert_eq!(
        rows(&items(&pr)),
        [(
            "thread",
            "Minor",
            "README.md:96",
            "Fix the grammar in the `/remove-ticket` description."
        )]
    );
}

#[test]
fn an_item_answered_with_the_marker_is_absent_and_back_after_a_human_reply() {
    let mut pr = fixture(PR52);
    thread(&mut pr, 0)["isResolved"] = json!(false);
    // the marker reply, then CodeRabbit's, here in REST's shape: a bot still
    let bot = &mut comments(thread(&mut pr, 0))[2];
    bot["author"] = json!({"login": "coderabbitai[bot]"});
    assert_eq!(items(&pr), [], "answered, only a bot after the marker");

    comments(thread(&mut pr, 0)).push(human(1, "Still slow on a big folder."));
    assert_eq!(
        rows(&items(&pr)),
        [(
            "thread",
            "Major",
            "src/orchestrator/cost.rs:153",
            "Bound session discovery on the shell thread."
        )]
    );
}

/// Answered `Covered by <id>: <title>.` with the marker, a PR comment is
/// not open, its thread resolved or not: what the merge reads.
#[test]
fn a_thread_answered_as_covered_by_another_ticket_is_absent() {
    let mut pr = fixture(PR78);
    for i in 0..2 {
        let answer = thread(&mut pr, i)["comments"]["nodes"][1]["body"].clone();
        let answer = answer.as_str().unwrap();
        assert!(answer.starts_with("Covered by harness-we9.3: "), "{answer}");
    }
    assert_eq!(items(&pr), [], "answered and resolved");

    for i in 0..2 {
        thread(&mut pr, i)["isResolved"] = json!(false);
    }
    assert_eq!(items(&pr), [], "the marker alone answers them");

    for i in 0..2 {
        comments(thread(&mut pr, i)).truncate(1);
    }
    assert_eq!(
        rows(&items(&pr)),
        [
            (
                "thread",
                "Minor",
                "docs/on-call.md:22",
                "Remove the unsupported Moshi notification claims."
            ),
            (
                "thread",
                "Minor",
                "docs/on-call.md:35",
                "Remove the Moshi setup instructions until the feature exists."
            ),
        ]
    );
}

#[test]
fn a_lgtm_review_body_entry_is_not_an_item_and_the_nitpick_beside_it_is() {
    let mut pr = fixture(PR37);
    assert_eq!(
        items(&pr),
        [],
        "the nitpick is answered by a marker PR comment"
    );

    comments(&mut pr).clear();
    let items = items(&pr);
    assert_eq!(
        rows(&items),
        [(
            "outside",
            "Trivial",
            "src/orchestrator/app.rs:315",
            "Write `set_typesafe` through `app::write`."
        )]
    );
    assert_eq!(items[0].id, "cr:3075189af7e9df7759cb8735");
    assert_eq!(items[0].author, "coderabbitai");
}

/// Only nitpicks and outside-diff entries are findings: a ♻️ duplicate
/// repeats a thread.
#[test]
fn a_duplicate_review_body_entry_is_not_an_item() {
    let mut pr = fixture(PR37);
    comments(&mut pr).clear();
    let review = &mut pr["reviews"]["nodes"][3]["body"];
    let body = review
        .as_str()
        .unwrap()
        .replace("🧹 Nitpick comments (1)", "♻️ Duplicate comments (1)");
    *review = json!(body);
    assert_eq!(items(&pr), []);
}

#[test]
fn a_greptile_outside_diff_item_is_rated_p1() {
    let items = items(&fixture(PR29));
    assert_eq!(
        rows(&items),
        [
            (
                "outside",
                "P1",
                "src/orchestrator/pipeline.rs:175",
                "Stage runs without snapshot"
            ),
            (
                "outside",
                "P1",
                "src/orchestrator/pipeline.rs:260",
                "Failed probe deletes concurrent work"
            ),
            (
                "outside",
                "P1",
                "",
                "Failed initial HEAD probe disables the read-only worktree guard"
            ),
            (
                "outside",
                "P1",
                "",
                "Transient tree-probe failure deletes concurrent work"
            ),
            (
                "outside",
                "P2",
                "src/orchestrator/pipeline.rs:172",
                "Failed probe saves null tree"
            ),
            (
                "outside",
                "P2",
                "",
                "Failed initial tree probe records null and parks a clean stage"
            ),
        ],
        "every thread is resolved and every human comment is an answer"
    );
    assert!(items.iter().all(|i| i.author == "greptile-apps"));
}

#[test]
fn a_greptile_summary_finding_is_an_item_only_without_its_thread() {
    let mut pr = fixture(PR34);
    let with_threads: Vec<_> = items(&pr).into_iter().map(|i| i.summary).collect();
    assert_eq!(
        with_threads,
        [
            "Instruction symlinks redirect writes",
            "Init follows repository symlinks when writing agent instructions and docs",
            "Claude file misses guidance",
            "Installed guide has broken pointer",
            "Moderator definition needs exception",
            "Installed issue-tracker guide points to an AGENTS.md that init does not create",
        ],
        "the summary's Findings are the threads'"
    );

    pr["reviewThreads"]["nodes"] = json!([]);
    let items = items(&pr);
    let summary: Vec<_> = rows(&items)
        .into_iter()
        .filter(|(_, _, _, s)| !s.starts_with("Init follows") && !s.starts_with("Installed issue"))
        .collect();
    assert_eq!(
        summary,
        [
            ("outside", "P1", "", "Instruction symlinks redirect writes"),
            ("outside", "P2", "", "Claude file misses guidance"),
            ("outside", "P2", "", "Installed guide has broken pointer"),
            ("outside", "P2", "", "Moderator definition needs exception"),
        ]
    );
}

#[test]
fn a_failed_check_run_is_a_check_item_rated_failed() {
    let mut pr = fixture(PR34);
    pr["statusCheckRollup"]["contexts"]["nodes"][0]["conclusion"] = json!("FAILURE");
    let items = items(&pr);
    assert_eq!(rows(&items)[0], ("check", "failed", "", "Greptile Review"));
    assert_eq!(items[0].author, "greptile-apps");
    assert_eq!(
        items[0].id, "check:034ef1009b9ecde2c299053934934b7d734e41e4:Greptile Review::",
        "a check run of no workflow"
    );
}

/// gh keeps the newest run of a check by name, workflow and event: a
/// failed run passed on a re-run is no item, and holds nothing.
#[test]
fn only_the_newest_run_of_a_check_counts() {
    let mut pr = fixture(PR34);
    let run = |conclusion: &str, started: &str| {
        json!({"name": "test", "status": "COMPLETED",
            "conclusion": conclusion, "startedAt": started,
            "checkSuite": {"app": {"slug": "github-actions"},
                "workflowRun": {"event": "pull_request", "workflow": {"name": "CI"}}}})
    };
    let contexts = &mut pr["statusCheckRollup"]["contexts"]["nodes"];
    *contexts = json!([
        run("FAILURE", "2026-09-28T10:00:00Z"),
        run("SUCCESS", "2026-09-28T10:05:00Z"),
    ]);
    let checks = |pr: &Value| -> Vec<Item> {
        items(pr)
            .into_iter()
            .filter(|i| i.kind == "check")
            .collect()
    };
    assert_eq!(checks(&pr), [], "re-run passed");

    let rerun = &mut pr["statusCheckRollup"]["contexts"]["nodes"][1];
    rerun["status"] = json!("IN_PROGRESS");
    rerun["conclusion"] = Value::Null;
    let reply: Pr = serde_json::from_value(pr.clone()).unwrap();
    assert!(reply.busy(), "the re-run is at work");

    pr["statusCheckRollup"]["contexts"]["nodes"][1] = run("FAILURE", "2026-09-28T10:05:00Z");
    let failed = checks(&pr);
    assert_eq!(failed.len(), 1, "one item for the check: {failed:?}");
    assert_eq!(failed[0].author, "github-actions");

    let mut queued = run("", "");
    queued["status"] = json!("QUEUED");
    queued["conclusion"] = Value::Null;
    queued["startedAt"] = Value::Null;
    let done = run("SUCCESS", "2026-09-28T10:05:00Z");
    for order in [[&done, &queued], [&queued, &done]] {
        pr["statusCheckRollup"]["contexts"]["nodes"] = json!(order);
        let reply: Pr = serde_json::from_value(pr.clone()).unwrap();
        assert!(
            reply.busy(),
            "a queued re-run, not started yet, is the newest"
        );
    }

    let mut skipped = run("SKIPPED", "");
    skipped["startedAt"] = Value::Null;
    let mut active = run("", "2026-09-28T10:05:00Z");
    active["status"] = json!("IN_PROGRESS");
    active["conclusion"] = Value::Null;
    let failure = run("FAILURE", "2026-09-28T10:05:00Z");
    for order in [[&skipped, &active], [&active, &skipped]] {
        pr["statusCheckRollup"]["contexts"]["nodes"] = json!(order);
        let reply: Pr = serde_json::from_value(pr.clone()).unwrap();
        assert!(reply.busy(), "a skipped run never started hides no re-run");
    }
    for order in [[&skipped, &failure], [&failure, &skipped]] {
        pr["statusCheckRollup"]["contexts"]["nodes"] = json!(order);
        assert_eq!(checks(&pr).len(), 1, "the re-run's failure is an item");
    }
}

#[test]
fn a_failed_status_is_a_check_item_by_its_creator() {
    let mut pr = fixture(PR65);
    let status = &mut pr["statusCheckRollup"]["contexts"]["nodes"][0];
    status["state"] = json!("ERROR");
    status["creator"] = json!({"login": "coderabbitai"});
    let items = items(&pr);
    assert_eq!(rows(&items), [("check", "failed", "", "CodeRabbit")]);
    assert_eq!(items[0].author, "coderabbitai");
}

#[test]
fn the_items_come_most_severe_first() {
    let mut pr = fixture(PR34);
    pr["statusCheckRollup"]["contexts"]["nodes"][0]["conclusion"] = json!("TIMED_OUT");
    let mut major = fixture(PR52)["reviewThreads"]["nodes"][0].clone();
    major["isResolved"] = json!(false);
    comments(&mut major).truncate(1);
    pr["reviewThreads"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(major);
    let nitpick = fixture(PR37)["reviews"]["nodes"][3].clone();
    pr["reviews"]["nodes"].as_array_mut().unwrap().push(nitpick);
    comments(&mut pr).push(human(7, "Please also cover the dry run."));

    let ratings: Vec<_> = items(&pr).into_iter().map(|i| i.rating).collect();
    assert_eq!(
        ratings,
        [
            "failed",
            "P1",
            "Major",
            "P1",
            "muresanroland",
            "P2",
            "P2",
            "P2",
            "P2",
            "Trivial",
        ]
    );
}

const URL: &str = "https://example.test/pr/hx-1";

/// The world, hx-1's PR open at URL, and its clock.
fn polled() -> (Arc<World>, Orchestrator, Arc<Mutex<DateTime<Local>>>) {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    let noon = Local.with_ymd_and_hms(2026, 9, 30, 12, 0, 0).unwrap();
    let clock = set_clock(&mut o.cfg, noon);
    o.update("hx-1", |ts| {
        ts.status = STATUS_PR_OPEN.to_string();
        ts.pr = URL.to_string();
    });
    (w, o, clock)
}

/// A fixture as an open PR whose head, and its rollup's commit, is `head`.
fn open(raw: &str, head: &str) -> Value {
    let mut pr = fixture(raw);
    pr["state"] = json!("OPEN");
    pr["mergeable"] = json!("MERGEABLE");
    pr["headRefOid"] = json!(head);
    pr["statusCheckRollup"]["commit"]["oid"] = json!(head);
    pr
}

fn serve(w: &World, pr: &Value) {
    w.lock().prs.insert(URL.to_string(), pr.to_string());
}

fn later(clock: &Mutex<DateTime<Local>>, seconds: i64) {
    *clock.lock().unwrap() += TimeDelta::seconds(seconds);
}

/// One poll: hx-1's new items if its head is quiet, None if not.
fn poll(o: &Orchestrator) -> Option<Vec<String>> {
    let quiet = o.poll_merges();
    assert!(quiet.iter().all(|(t, _)| t == "hx-1"), "{quiet:?}");
    quiet
        .into_iter()
        .next()
        .map(|(_, items)| items.into_iter().map(|i| i.id).collect())
}

#[test]
fn a_new_head_resets_the_minute() {
    let (w, o, clock) = polled();
    serve(&w, &open(PR65, "a"));
    assert_eq!(poll(&o), None, "a head first seen");
    later(&clock, 59);
    assert_eq!(poll(&o), None, "59s on");

    serve(&w, &open(PR65, "b"));
    later(&clock, 1);
    assert_eq!(poll(&o), None, "b is new");
    later(&clock, 59);
    assert_eq!(poll(&o), None, "59s into b");
    later(&clock, 1);
    assert_eq!(poll(&o), Some(vec![]), "b held 60s, nothing open");
}

#[test]
fn sixty_seconds_on_with_nothing_pending_the_head_is_quiet() {
    let (w, o, clock) = polled();
    let mut pr = open(PR65, "a");
    thread(&mut pr, 0)["isResolved"] = json!(false);
    serve(&w, &pr);
    assert_eq!(poll(&o), None);
    later(&clock, 60);
    let id = "PRRT_kwDOUiwtFs6meF8y".to_string();
    assert_eq!(poll(&o), Some(vec![id.clone()]));
    assert_eq!(poll(&o), Some(vec![]), "offered once");
    assert_eq!(o.ticket("hx-1").offered, BTreeSet::from([id]));
}

#[test]
fn a_pending_status_context_keeps_the_head_busy() {
    let (w, o, clock) = polled();
    let mut pr = open(PR65, "a");
    let status = &mut pr["statusCheckRollup"]["contexts"]["nodes"][0];
    assert_eq!(status["context"], "CodeRabbit");
    status["state"] = json!("PENDING");
    serve(&w, &pr);
    poll(&o);
    later(&clock, 600);
    assert_eq!(poll(&o), None, "CodeRabbit is still reviewing");

    pr["statusCheckRollup"]["contexts"]["nodes"][0]["state"] = json!("SUCCESS");
    serve(&w, &pr);
    assert_eq!(poll(&o), Some(vec![]));
}

#[test]
fn a_greptile_context_only_on_an_earlier_commit_does_not_hold_it() {
    let (w, o, clock) = polled();
    serve(&w, &open(PR34, "a"));
    poll(&o);
    later(&clock, 60);
    assert!(poll(&o).is_some(), "a is quiet");

    // b: the rollup still names a, where Greptile was at work
    let mut pr = open(PR34, "b");
    pr["statusCheckRollup"]["commit"]["oid"] = json!("a");
    pr["statusCheckRollup"]["contexts"]["nodes"][0]["status"] = json!("IN_PROGRESS");
    serve(&w, &pr);
    assert_eq!(poll(&o), None, "b is new");
    later(&clock, 60);
    assert!(poll(&o).is_some(), "Greptile does not re-review b");

    pr["statusCheckRollup"] = Value::Null; // no context on b at all
    serve(&w, &pr);
    assert!(poll(&o).is_some());
}

/// Rebase goes before PR comments: a quiet head is offered nothing while
/// its PR conflicts, or has a Rebase queued or running, and its items wait.
#[test]
fn a_conflicting_pr_or_one_with_a_rebase_queued_or_running_is_offered_nothing() {
    let (w, o, clock) = polled();
    let mut pr = open(PR65, "a");
    thread(&mut pr, 0)["isResolved"] = json!(false);
    pr["mergeable"] = json!("CONFLICTING");
    serve(&w, &pr);
    poll(&o);
    later(&clock, 60);
    assert_eq!(poll(&o), None, "conflicting");

    pr["mergeable"] = json!("MERGEABLE");
    serve(&w, &pr);
    o.command("rebase-hx-1");
    assert_eq!(poll(&o), None, "a Rebase queued");
    o.consume("rebase-hx-1");
    let active = || o.active.lock().unwrap();
    active().insert("hx-1".to_string(), Some(REBASE.name));
    assert_eq!(poll(&o), None, "a Rebase running");
    active().clear();
    let id = "PRRT_kwDOUiwtFs6meF8y".to_string();
    assert_eq!(poll(&o), Some(vec![id]), "offered once the Rebase is done");
}

/// Parked at its PR Stage, the Ticket's PR is still polled for its merge,
/// which closes it, but nothing starts on it: its conflict is neither said
/// nor rebased, and no item is offered.
#[test]
fn a_ticket_parked_at_its_pr_stage_is_offered_nothing_and_its_merge_closes_it() {
    let (w, o, clock) = polled();
    set_switch(&w.repo, &REBASE_AUTO, true).unwrap();
    o.update("hx-1", |ts| {
        ts.status = STATUS_PARKED.to_string();
        ts.stage = REBASE.name.to_string();
        ts.reason = AWAY.to_string();
    });
    let mut pr = open(PR65, "a");
    thread(&mut pr, 0)["isResolved"] = json!(false);
    pr["mergeable"] = json!("CONFLICTING");
    serve(&w, &pr);
    poll(&o);
    later(&clock, 60);
    assert_eq!(poll(&o), None, "offered while parked");
    assert!(o.commands().is_empty(), "{:?}", o.commands());
    let said = w.lines();
    assert!(
        !said.iter().any(|l| l.contains("conflicts with main")),
        "{said:?}"
    );

    pr["state"] = json!("MERGED");
    serve(&w, &pr);
    poll(&o);
    assert_eq!(o.ticket("hx-1").status, STATUS_MERGED);
    let close = format!("bd close hx-1 --reason PR merged: {URL}");
    assert_eq!(w.called(&close).len(), 1);
}

/// A Ticket parked because its PR closed unmerged is not polled again,
/// though a PR Stage was the last it ran: nor once /continue has it back
/// in its Pipeline, which clears the reason.
#[test]
fn a_ticket_parked_because_its_pr_closed_is_not_polled_again() {
    let (w, o, _) = polled();
    o.update("hx-1", |ts| ts.stage = REBASE.name.to_string()); // rebased once
    let mut pr = open(PR65, "a");
    pr["state"] = json!("CLOSED");
    serve(&w, &pr);
    poll(&o);
    assert_eq!(o.ticket("hx-1").status, STATUS_PARKED);

    let asked = w.called("gh ").len();
    poll(&o);
    assert_eq!(w.called("gh ").len(), asked, "a closed PR polled again");
    o.update("hx-1", |ts| {
        ts.status = STATUS_RUNNING.to_string();
        ts.reason.clear();
    });
    poll(&o);
    assert_eq!(w.called("gh ").len(), asked, "polled in its Pipeline");
}

/// A Ticket parked at its PR Stage while gh is asked about its PR is
/// taken as parked: nothing offered, its conflict neither said nor rebased.
#[test]
fn a_ticket_parked_during_the_poll_is_offered_nothing_nor_rebased() {
    let (w, o, clock) = polled();
    set_switch(&w.repo, &REBASE_AUTO, true).unwrap();
    let o = Arc::new(o);
    let armed = Arc::new(AtomicBool::new(false));
    let (orq, arm) = (Arc::downgrade(&o), armed.clone());
    w.hook(move |_, argv| {
        if argv.starts_with(&["gh", "api", "graphql"]) && arm.swap(false, Ordering::SeqCst) {
            orq.upgrade()?.update("hx-1", |ts| {
                ts.status = STATUS_PARKED.to_string();
                ts.stage = REBASE.name.to_string();
            });
        }
        None
    });
    let mut pr = open(PR65, "a");
    thread(&mut pr, 0)["isResolved"] = json!(false);
    serve(&w, &pr);
    poll(&o);
    later(&clock, 60);
    armed.store(true, Ordering::SeqCst);
    assert_eq!(poll(&o), None, "offered while parked");
    assert!(o.ticket("hx-1").offered.is_empty());

    o.update("hx-1", |ts| ts.status = STATUS_PR_OPEN.to_string());
    pr["mergeable"] = json!("CONFLICTING");
    serve(&w, &pr);
    armed.store(true, Ordering::SeqCst);
    poll(&o);
    let ts = o.ticket("hx-1");
    assert!(!ts.conflict && !ts.conflicting, "conflict recorded");
    assert!(o.commands().is_empty(), "{:?}", o.commands());
}

/// Its dependents wait on its merge, said once, only when the PR is
/// settled: a quiet head, no conflict, nothing new offered, no Address PR
/// comments approved or running.
#[test]
fn dependents_wait_on_the_merge_once_the_pr_is_settled() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1"), with_deps("hx-2", &["hx-1"])]);
    let clock = set_clock(
        &mut o.cfg,
        Local.with_ymd_and_hms(2026, 9, 30, 12, 0, 0).unwrap(),
    );
    o.update("hx-1", |ts| {
        ts.status = STATUS_PR_OPEN.to_string();
        ts.pr = URL.to_string();
    });
    o.change_state(|s| s.queue = vec!["hx-1".to_string(), "hx-2".to_string()]);
    let settled = || o.ticket("hx-1").settled;
    let waiting = || {
        (w.lines().iter())
            .filter(|l| l.starts_with("hx-2 waiting for PR #hx-1 to merge"))
            .count()
    };

    let mut pr = open(PR65, "a");
    thread(&mut pr, 0)["isResolved"] = json!(false);
    serve(&w, &pr);
    poll(&o);
    assert!(!settled(), "a head first seen");
    later(&clock, 60);
    assert_eq!(poll(&o).map(|ids| ids.len()), Some(1));
    assert!(!settled(), "a thread just offered");
    o.approve_comments("hx-1", Vec::new(), Vec::new(), false);
    poll(&o);
    assert!(!settled(), "Address PR comments approved");
    o.approved.lock().unwrap().clear();
    let active = || o.active.lock().unwrap();
    active().insert("hx-1".to_string(), Some(ADDRESS_PR_COMMENTS.name));
    poll(&o);
    assert!(!settled(), "Address PR comments running");
    active().clear();
    assert_eq!(waiting(), 0);

    // its push resolved the thread
    serve(&w, &open(PR65, "b"));
    poll(&o);
    assert!(!settled(), "b is new");
    later(&clock, 60);
    poll(&o);
    poll(&o);
    assert!(settled());
    assert_eq!(waiting(), 1, "{:#?}", w.lines());

    pr = open(PR65, "b");
    pr["mergeable"] = json!("CONFLICTING");
    serve(&w, &pr);
    poll(&o);
    assert!(!settled(), "conflicting");
}

/// Agent merge on, with `bots` the repo's review bots.
fn agent_merge(w: &World, bots: &[&str]) {
    set_switch(&w.repo, &ADDRESS_PR_COMMENTS_AUTO, true).unwrap();
    set_switch(&w.repo, &AGENT_MERGE, true).unwrap();
    set_review_bots(&w.repo, bots).unwrap();
}

/// Polls `pr` until its head has held a minute: the poll that may merge it.
fn settle(w: &World, o: &Orchestrator, clock: &Mutex<DateTime<Local>>, pr: &Value) {
    serve(w, pr);
    poll(o);
    later(clock, 60);
    poll(o);
}

/// The merges asked of gh so far.
fn merges(w: &World) -> Vec<String> {
    w.called("gh pr merge")
}

/// hx-1's branch, from main to head `a`, changed these paths.
fn changed(w: &World, paths: &'static str) {
    w.hook(move |_, argv| {
        let diff = "git diff --no-renames --name-only -z origin/main...a";
        (argv.join(" ") == diff).then(|| Ok(paths.to_string()))
    });
}

/// Under Agent merge a reviewed PR, its bot's review in, every PR comment
/// answered and its checks green, is merged once with the repo's method; the
/// next poll's merged handling closes the Ticket.
#[test]
fn a_reviewed_pr_all_answered_and_green_merges_once_then_its_ticket_closes() {
    let (w, o, clock) = polled();
    agent_merge(&w, &["coderabbit"]);
    settle(&w, &o, &clock, &open(PR65, "a"));
    let merge =
        format!("gh pr merge {URL} --squash --delete-branch --match-head-commit a --repo o/r");
    assert_eq!(merges(&w), [merge]);
    let said = w.lines();
    assert!(
        said.contains(&"hx-1 PR #hx-1 merged by Orqadence".to_string()),
        "{said:?}"
    );
    assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN, "the next poll's");

    poll(&o);
    assert_eq!(o.ticket("hx-1").status, STATUS_MERGED);
    assert_eq!(w.called("bd close hx-1").len(), 1);
    poll(&o);
    assert_eq!(merges(&w).len(), 1);
}

/// Agent merge off, the Orchestrator never merges: a human does (ADR 0002).
#[test]
fn agent_merge_off_never_merges() {
    let (w, o, clock) = polled();
    set_review_bots(&w.repo, &["coderabbit"]).unwrap();
    settle(&w, &o, &clock, &open(PR65, "a"));
    assert!(o.ticket("hx-1").settled);
    assert!(merges(&w).is_empty());
}

/// A Ticket whose PR is human-merge is never merged, even under Agent merge.
#[test]
fn a_human_merge_pr_is_never_merged() {
    let (w, o, clock) = polled();
    agent_merge(&w, &["coderabbit"]);
    o.update("hx-1", |ts| ts.human_merge = true);
    settle(&w, &o, &clock, &open(PR65, "a"));
    assert!(o.ticket("hx-1").settled);
    assert!(merges(&w).is_empty());
}

/// A review by `login`, as the poll's reply holds it.
fn review(login: &str) -> Value {
    json!({"databaseId": 7, "author": {"login": login, "__typename": "Bot"}, "body": ""})
}

/// A human's changes-requested review blocks the merge until it is gone.
#[test]
fn changes_requested_blocks_the_merge() {
    let (w, o, clock) = polled();
    agent_merge(&w, &["coderabbit"]);
    let mut pr = open(PR65, "a");
    pr["reviewDecision"] = json!("CHANGES_REQUESTED");
    settle(&w, &o, &clock, &pr);
    assert!(merges(&w).is_empty());

    pr["reviewDecision"] = json!("APPROVED");
    serve(&w, &pr);
    poll(&o);
    assert_eq!(merges(&w).len(), 1);
}

/// A reviewed PR waits for a review from each bot review_bots lists, on any
/// commit, and for every PR comment to be answered.
#[test]
fn a_reviewed_pr_waits_for_each_listed_bot_and_for_its_open_items() {
    let (w, o, clock) = polled();
    agent_merge(&w, &["coderabbit", "greptile"]);
    let mut pr = open(PR65, "a");
    settle(&w, &o, &clock, &pr);
    assert!(o.ticket("hx-1").settled);
    assert!(merges(&w).is_empty(), "Greptile has not reviewed");

    let reviews = pr["reviews"]["nodes"].as_array_mut().unwrap();
    reviews.push(review("greptile-apps"));
    thread(&mut pr, 0)["isResolved"] = json!(false);
    serve(&w, &pr);
    poll(&o); // offers the thread
    poll(&o);
    assert!(o.ticket("hx-1").settled);
    assert!(merges(&w).is_empty(), "a thread is open");

    thread(&mut pr, 0)["isResolved"] = json!(true);
    serve(&w, &pr);
    poll(&o);
    assert_eq!(merges(&w).len(), 1);
}

/// A No-review pull request waits for no bot: it merges once its checks
/// are green and GitHub calls it mergeable.
#[test]
fn a_no_review_pr_merges_on_green_without_its_bots() {
    let (w, o, clock) = polled();
    agent_merge(&w, &["coderabbit", "greptile"]);
    o.update("hx-1", |ts| ts.no_review = true);
    changed(&w, "README.md\0");
    let mut pr = open(PR65, "a");
    pr["reviews"]["nodes"] = json!([]);
    pr["statusCheckRollup"]["contexts"]["nodes"][0]["state"] = json!("FAILURE");
    settle(&w, &o, &clock, &pr);
    poll(&o);
    assert!(o.ticket("hx-1").settled);
    assert!(merges(&w).is_empty(), "a check failed");

    pr["statusCheckRollup"]["contexts"]["nodes"][0]["state"] = json!("SUCCESS");
    pr["mergeable"] = json!("UNKNOWN");
    serve(&w, &pr);
    poll(&o);
    assert!(o.ticket("hx-1").settled);
    assert!(merges(&w).is_empty(), "GitHub has not said it is mergeable");

    pr["mergeable"] = json!("MERGEABLE");
    serve(&w, &pr);
    poll(&o);
    assert_eq!(merges(&w).len(), 1);
}

/// A No-review PR whose head now changes a source file is one no longer:
/// orqa:no-review comes off, so the bots review it, and it waits for them.
#[test]
fn a_no_review_pr_that_changes_a_source_file_since_loses_its_exemption() {
    let (w, o, clock) = polled();
    agent_merge(&w, &["greptile"]);
    o.update("hx-1", |ts| ts.no_review = true);
    changed(&w, "README.md\0src/main.rs\0");
    settle(&w, &o, &clock, &open(PR65, "a"));
    assert!(merges(&w).is_empty());
    assert!(!o.ticket("hx-1").no_review);
    assert_eq!(
        w.called("gh pr edit"),
        [format!("gh pr edit {URL} --remove-label orqa:no-review")]
    );
    poll(&o);
    assert!(merges(&w).is_empty(), "Greptile has not reviewed");
}

/// A No-review PR whose orqa:no-review gh failed to remove stays No-review,
/// so it is not merged and the next poll removes the label again.
#[test]
fn a_no_review_label_gh_failed_to_remove_is_removed_on_the_next_poll() {
    let (w, o, clock) = polled();
    agent_merge(&w, &["greptile"]);
    o.update("hx-1", |ts| ts.no_review = true);
    changed(&w, "src/main.rs\0");
    w.fail_once("gh pr edit", "HTTP 502");
    settle(&w, &o, &clock, &open(PR65, "a"));
    assert!(merges(&w).is_empty());
    assert!(o.ticket("hx-1").no_review);

    poll(&o);
    assert!(merges(&w).is_empty());
    assert!(!o.ticket("hx-1").no_review);
    let remove = format!("gh pr edit {URL} --remove-label orqa:no-review");
    assert_eq!(w.called("gh pr edit"), [remove.clone(), remove]);
}

/// A reviewed PR with more threads, thread comments, reviews or PR
/// comments than the poll reads is not merged: an open one may be past them.
#[test]
fn a_pr_with_items_past_the_poll_page_is_not_merged() {
    for (path, page) in [
        ("/reviewThreads/pageInfo", json!({"hasNextPage": true})),
        (
            "/reviewThreads/nodes/0/comments/pageInfo",
            json!({"hasNextPage": true}),
        ),
        ("/reviews/pageInfo", json!({"hasPreviousPage": true})),
        ("/comments/pageInfo", json!({"hasPreviousPage": true})),
    ] {
        let (w, o, clock) = polled();
        agent_merge(&w, &["coderabbit"]);
        let mut pr = open(PR65, "a");
        let (parent, key) = path.rsplit_once('/').unwrap();
        pr.pointer_mut(parent).unwrap()[key] = page;
        settle(&w, &o, &clock, &pr);
        assert!(merges(&w).is_empty(), "{path}");

        pr.pointer_mut(parent).unwrap()[key] = json!({});
        serve(&w, &pr);
        poll(&o);
        assert_eq!(merges(&w).len(), 1, "{path}");
    }
}

/// A Ticket whose labels make it human-merge is not merged though its
/// state says otherwise, as one saved before human_merge was does: the
/// labels are read again before a merge, and the state and PR keep it.
#[test]
fn a_human_merge_ticket_is_not_merged_whatever_its_saved_state_says() {
    let (w, o, clock) = polled();
    agent_merge(&w, &["coderabbit"]);
    w.lock().tickets[0].labels = vec!["orqa:human-merge".to_string()];
    settle(&w, &o, &clock, &open(PR65, "a"));
    assert!(merges(&w).is_empty());
    assert!(o.ticket("hx-1").human_merge);
    assert_eq!(
        w.called("gh pr edit"),
        [format!("gh pr edit {URL} --add-label orqa:human-merge")]
    );
}

/// A head with more checks than the poll reads is never green.
#[test]
fn checks_past_the_first_hundred_are_not_green() {
    let (w, o, clock) = polled();
    agent_merge(&w, &["coderabbit"]);
    let mut pr = open(PR65, "a");
    pr["statusCheckRollup"]["contexts"]["pageInfo"] = json!({"hasNextPage": true});
    settle(&w, &o, &clock, &pr);
    assert!(merges(&w).is_empty());

    pr["statusCheckRollup"]["contexts"]["pageInfo"] = json!({"hasNextPage": false});
    serve(&w, &pr);
    poll(&o);
    assert_eq!(merges(&w).len(), 1);
}

/// hx-1's PR merged in a repo whose settings allow `squash` and `rebase`:
/// the method gh was given.
fn merged_with(squash: bool, rebase: bool) -> String {
    let (w, o, clock) = polled();
    agent_merge(&w, &["coderabbit"]);
    w.hook(move |_, argv| {
        let settings = json!({"full_name": "o/r", "allow_squash_merge": squash,
            "allow_rebase_merge": rebase, "allow_merge_commit": true});
        (argv == ["gh", "api", "repos/{owner}/{repo}"]).then(|| Ok(settings.to_string()))
    });
    settle(&w, &o, &clock, &open(PR65, "a"));
    let merges = merges(&w);
    assert_eq!(merges.len(), 1, "{merges:?}");
    let method = merges[0].split(' ').find(|arg| arg.starts_with("--"));
    method.unwrap().to_string()
}

/// The merge takes the repo's own method: squash, else rebase, else a
/// merge commit.
#[test]
fn the_merge_takes_the_repos_method_squash_else_rebase_else_a_merge_commit() {
    assert_eq!(merged_with(true, true), "--squash");
    assert_eq!(merged_with(false, true), "--rebase");
    assert_eq!(merged_with(false, false), "--merge");
}

const REFUSAL: &str =
    "Pull request o/r#1 is not mergeable: the base branch policy prohibits the merge.";

/// hx-1 parked by a merge GitHub refused.
fn refused() -> (Arc<World>, Orchestrator, Value) {
    let (w, o, clock) = polled();
    agent_merge(&w, &["coderabbit"]);
    w.fail_once("gh pr merge", REFUSAL);
    let pr = open(PR65, "a");
    settle(&w, &o, &clock, &pr);
    (w, o, pr)
}

/// GitHub refusing the merge parks the Ticket with gh's message. Its PR
/// stays open, still polled: nothing is asked of it again, and merged by
/// hand it closes its Ticket.
#[test]
fn a_merge_github_refuses_parks_the_ticket_with_ghs_message_its_pr_still_polled() {
    let (w, o, mut pr) = refused();
    let ts = o.ticket("hx-1");
    let reason = format!("PR #hx-1 not merged: {REFUSAL}");
    assert_eq!((ts.status.as_str(), &ts.reason), (STATUS_PARKED, &reason));
    let said = w.lines();
    assert!(said.contains(&format!("hx-1 parked: {reason}")), "{said:?}");
    poll(&o);
    assert_eq!(merges(&w).len(), 1, "asked again while parked");

    pr["state"] = json!("MERGED");
    serve(&w, &pr);
    poll(&o);
    assert_eq!(o.ticket("hx-1").status, STATUS_MERGED);
}

/// /continue, and /retry, give a Ticket a refused merge parked back to the
/// poll, never to its Pipeline: the merge is tried again.
#[test]
fn continue_gives_a_ticket_parked_by_a_refused_merge_back_to_the_poll() {
    for command in ["continue-hx-1", "retry-hx-1"] {
        let (w, o, _) = refused();
        let o = Arc::new(o);
        let mut run = spawn_epic(o.clone(), "hx");
        o.command(command);
        w.await_line("hx-1 merged, Ticket closed");
        run.wait();
        o.wait_in_flight();
        assert_eq!(merges(&w).len(), 2, "{command}");
        assert!(w.called("herdr agent start").is_empty(), "{command}");
    }
}

/// gh failing without a word of its own parks with what failed.
#[test]
fn a_merge_that_fails_without_a_message_parks_with_the_command_that_failed() {
    let (w, o, clock) = polled();
    agent_merge(&w, &["coderabbit"]);
    w.fail_once("gh pr merge", "");
    settle(&w, &o, &clock, &open(PR65, "a"));
    let reason = o.ticket("hx-1").reason;
    assert!(
        reason.starts_with("PR #hx-1 not merged: gh pr merge ")
            && reason.ends_with("exit status 1: "),
        "{reason}"
    );
}

/// A PR whose base branch has a merge queue is GitHub's to merge: gh would
/// queue it, so it is not asked, and the Ticket parks.
#[test]
fn a_pr_whose_base_has_a_merge_queue_parks_and_gh_is_not_asked() {
    let (w, o, clock) = polled();
    agent_merge(&w, &["coderabbit"]);
    let mut pr = open(PR65, "a");
    pr["isMergeQueueEnabled"] = json!(true);
    settle(&w, &o, &clock, &pr);
    let ts = o.ticket("hx-1");
    let reason = "PR #hx-1 not merged: its base branch has a merge queue";
    assert_eq!(
        (ts.status.as_str(), ts.reason.as_str()),
        (STATUS_PARKED, reason)
    );
    assert!(merges(&w).is_empty());
}

/// Nothing merges before the PR is settled: not in its head's quiet minute,
/// nor with Address PR comments approved or running, even a No-review PR.
#[test]
fn a_pr_is_not_merged_before_it_is_settled() {
    let (w, o, clock) = polled();
    agent_merge(&w, &[]);
    o.update("hx-1", |ts| ts.no_review = true);
    changed(&w, "README.md\0");
    serve(&w, &open(PR65, "a"));
    poll(&o);
    later(&clock, 59);
    poll(&o);
    assert!(merges(&w).is_empty(), "59s into its head");

    later(&clock, 1);
    o.approve_comments("hx-1", Vec::new(), Vec::new(), false);
    poll(&o);
    assert!(merges(&w).is_empty(), "Address PR comments approved");
    o.approved.lock().unwrap().clear();
    let active = || o.active.lock().unwrap();
    active().insert("hx-1".to_string(), Some(ADDRESS_PR_COMMENTS.name));
    poll(&o);
    assert!(merges(&w).is_empty(), "Address PR comments running");

    active().clear();
    poll(&o);
    assert_eq!(merges(&w).len(), 1);
}

/// A merge gh took is not asked twice while GitHub still shows the PR open.
#[test]
fn a_merge_gh_took_is_not_asked_twice_while_the_pr_reads_open() {
    let (w, o, clock) = polled();
    agent_merge(&w, &["coderabbit"]);
    w.hook(|_, argv| {
        argv.starts_with(&["gh", "pr", "merge"])
            .then(|| Ok(String::new()))
    });
    settle(&w, &o, &clock, &open(PR65, "a"));
    poll(&o);
    assert_eq!(o.ticket("hx-1").status, STATUS_PR_OPEN);
    assert_eq!(merges(&w).len(), 1);
}
