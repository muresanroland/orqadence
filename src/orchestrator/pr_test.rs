//! The PR poll's reply: its items over trimmed real replies of this repo's
//! PRs, and the quiet head on the world's clock.

use chrono::{DateTime, Local, TimeDelta, TimeZone};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use super::pr::{Item, Pr};
use super::stage::Orchestrator;
use super::state::STATUS_PR_OPEN;
use super::world::{new_world, set_clock, BdTicket, World};

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
        json!({"__typename": "CheckRun", "name": "test", "status": "COMPLETED",
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
