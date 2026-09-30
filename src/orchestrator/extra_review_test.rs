//! The Extra review: the Stage an Area label adds after the Review, on the
//! label's row, every Round or in Round 1 only, its Findings into the
//! Debate or straight to the Fix; Limited like Implement, skipped with an
//! unreviewed Round.

use super::limit_test::{hits, CODEX};
use super::pipeline_test::stages_run;
use super::stage::{Answer, Ask, Orchestrator, StageError, AWAY, EXTRA_REVIEW};
use super::state::{Review, Session, STATUS_PARKED};
use super::world::{new_world, set_clock, spawn_ticket, succeed, wait_until, BdTicket, World};
use super::write_file;
use chrono::TimeZone;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

/// hx-1 with the bd labels given.
fn labelled_ticket(labels: &[&str]) -> BdTicket {
    BdTicket {
        labels: labels.iter().map(|l| l.to_string()).collect(),
        ..BdTicket::new("hx-1")
    }
}

/// config.json with orqa:security, an Area label whose Extra review is
/// `extra` (its fields over the defaults), and orqa:codex-review, a
/// Modifier; the Extra review's skill installed.
fn config(w: &World, extra: Value) {
    labels(w, extra);
    w.installed("orqa-sec-review");
}

/// config's labels alone.
fn labels(w: &World, extra: Value) {
    let mut review = json!({"skill": "orqa-sec-review"});
    for (k, v) in extra.as_object().unwrap() {
        review[k] = v.clone();
    }
    let doc = json!({"labels": {
        "security": {"kind": "area", "extra_review": review},
        "codex-review": {"kind": "modifier"},
    }});
    write_file(&w.repo.join(".orqadence/config.json"), &doc.to_string());
}

/// A Finding a Review-shaped Stage returns, and a Verdict fixing or
/// skipping it.
const FOUND: &str = "STATUS: done\n\n## Findings\n\n- (medium) a.go:1 — x\n";
const FIX: &str =
    "STATUS: done\n- [fix] (medium) a.go:1 — x | reason: agreed | settled: consensus\n";
const SKIP: &str =
    "STATUS: done\n- [skip] (medium) a.go:1 — x | reason: fine | settled: consensus\n";

/// The Extra review runs after the Review in every Round, on the label's
/// App, model and effort, as stage-review with the label's skill in place
/// of the review pick, read-only in the Run directory, and says what it
/// found. An empty field of its row falls back to the Review's.
#[test]
fn an_area_labels_extra_review_runs_after_the_review_each_round_on_its_row() {
    let (w, o) = new_world(vec![labelled_ticket(&[
        "orqa:codex-review",
        "orqa:security",
    ])]);
    config(
        &w,
        json!({"app": "claude", "model": "opus", "effort": "high"}),
    );
    // its Finding fixed in Round 1, so a second Round runs, skipped in Round 2
    w.session(|p| match (p.stage.as_str(), p.round) {
        ("extra-review", _) => (FOUND.to_string(), "idle".to_string()),
        ("verdict", 1) => (FIX.to_string(), "idle".to_string()),
        ("verdict", _) => (SKIP.to_string(), "idle".to_string()),
        _ => succeed(p),
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    w.await_line("hx-1 PR #hx-1 opened after 2 rounds");
    run.wait();
    assert_eq!(
        stages_run(&w),
        [
            "implement",
            "review",
            "extra-review",
            "debate",
            "fix",
            "review",
            "extra-review",
            "debate",
            "fix"
        ]
    );
    w.await_line("hx-1 extra review 1 started: claude opus/high (pane 1-3)");
    w.await_line("hx-1 extra review 1 found 1 finding");
    w.await_line("hx-1 extra review 2 found 1 finding");
    let prompt = w.prompt("extra-review-1.md");
    assert!(prompt.contains("# Review Stage"), "{prompt}");
    assert!(
        prompt.contains("Use the orqa-sec-review skill for this review"),
        "{prompt}"
    );
    assert!(!prompt.contains("Label"), "{prompt}");
    let start = w.called("herdr agent start h-hx-1-extra-review ").remove(0);
    assert!(
        start.contains("--kind claude") && start.contains("--model opus"),
        "{start}"
    );
    let splits = w.called("herdr pane split");
    let run_dir = format!("--cwd {}", o.run_dir("hx-1").display());
    assert!(splits[1].contains(&run_dir), "{}", splits[1]);
    // no fetch.sh, no fetch
    assert!(w.called("env ").is_empty(), "{:?}", w.called("env "));
    assert!(!prompt.contains("- Cache:"), "{prompt}");
    // the Fix that opens the PR gets every Extra review file, for the PR
    let dir = o.run_dir("hx-1");
    let files = format!(
        "- Extra review files: {}, {}\n",
        dir.join("extra-review-1.md").display(),
        dir.join("extra-review-2.md").display()
    );
    let fix = w.prompt("fix-2.md");
    assert!(fix.contains(&files), "{fix}");
    assert!(!w.prompt("fix-1.md").contains("- Extra review files:"));

    // an empty row falls back to the Review's: codex, from config.json
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    config(&w, json!({}));
    o.run_ticket("hx-1");
    w.await_line("hx-1 extra review 1 started: codex (pane 1-3)");
    w.await_line("hx-1 PR #hx-1 opened");
}

/// orqa-sec-review ships a fetch.sh, which a Ticket's worktree gets from
/// the checkout; the base is origin/main, and every fetch call is kept
/// with the directory it ran in.
fn fetch_sh(w: &Arc<World>) -> Arc<Mutex<Vec<(PathBuf, String)>>> {
    let script = w.repo.join(".orqadence/skills/orqa-sec-review/fetch.sh");
    write_file(&script, "#!/usr/bin/env bash\n");
    let fetches = Arc::new(Mutex::new(Vec::new()));
    let kept = fetches.clone();
    w.hook(move |dir, argv| match argv {
        ["git", "symbolic-ref", ..] => Some(Ok("origin/main\n".to_string())),
        ["env", ..] => {
            kept.lock()
                .unwrap()
                .push((dir.to_path_buf(), argv.join(" ")));
            None
        }
        _ => None,
    });
    fetches
}

/// The fetch call for hx-1's worktree, as the Orchestrator makes it.
fn fetch_call(o: &Orchestrator) -> String {
    format!(
        "env ORQA_CACHE={} ORQA_BASE=origin/main bash {}",
        o.cfg.repo.join(".orqadence-local/cache/security").display(),
        o.worktree("hx-1")
            .join(".orqadence/skills/orqa-sec-review/fetch.sh")
            .display()
    )
}

/// Where each call starting with `prefix` is in the call order.
fn call_positions(w: &World, prefix: &str) -> Vec<usize> {
    let calls = w.calls();
    (0..calls.len())
        .filter(|&i| calls[i].starts_with(prefix))
        .collect()
}

/// A label whose Extra review skill has a fetch.sh runs it, in the worktree
/// with its cache and the base, before the Extra review's pane starts,
/// every Round and before the PR; the Extra review gets the cache as an
/// Input.
#[test]
fn a_labels_fetch_sh_runs_before_each_extra_review_with_the_cache_and_base() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    config(&w, json!({}));
    let fetches = fetch_sh(&w);
    w.session(|p| match (p.stage.as_str(), p.round) {
        ("verdict", 1) => (FIX.to_string(), "idle".to_string()),
        _ => succeed(p),
    });
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened after 2 rounds");
    let call = fetch_call(&o);
    assert_eq!(
        *fetches.lock().unwrap(),
        [
            (o.worktree("hx-1"), call.clone()),
            (o.worktree("hx-1"), call)
        ]
    );
    let (fetched, started) = (
        call_positions(&w, "env "),
        call_positions(&w, "herdr agent start h-hx-1-extra-review"),
    );
    assert!(
        fetched.len() == 2
            && fetched[0] < started[0]
            && started[0] < fetched[1]
            && fetched[1] < started[1],
        "fetch calls at {fetched:?}, extra reviews started at {started:?}"
    );
    let cache = o.cfg.repo.join(".orqadence-local/cache/security");
    assert!(cache.is_dir());
    let prompt = w.prompt("extra-review-1.md");
    assert!(
        prompt.contains(&format!("- Cache: {}\n", cache.display())),
        "{prompt}"
    );
    assert!(!prompt.contains("- Fetch:"), "{prompt}");

    let (w, o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    config(&w, json!({"position": "before_pr"}));
    let fetches = fetch_sh(&w);
    o.run_ticket("hx-1");
    w.await_line("hx-1 PR #hx-1 opened after 1 round");
    assert_eq!(fetches.lock().unwrap().len(), 1);
    assert!(
        call_positions(&w, "env ")[0]
            < call_positions(&w, "herdr agent start h-hx-1-extra-review")[0]
    );
    assert!(w.prompt("extra-review-final.md").contains("- Cache: "));
}

/// A fetch.sh that fails with this stderr, and the error it makes: its
/// last three lines that are not blank.
const FAILED: &str = "Initializing provider plugins...\n\nError: Failed to query available provider packages\nregistry.terraform.io: dial tcp: lookup failed\n  exit 1\n";
const ERROR: &str = "Error: Failed to query available provider packages / registry.terraform.io: dial tcp: lookup failed / exit 1";

/// fetch_sh's world with every fetch failing with FAILED.
fn failing_fetch(w: &Arc<World>) {
    fetch_sh(w);
    w.hook(|_, argv| match argv {
        ["env", ..] => Some(Err(FAILED.to_string())),
        _ => None,
    });
}

/// A failing fetch is a Question with its stderr's last lines: retry runs
/// it again; run without starts the Extra review with the Fetch Input, the
/// Cache still given.
#[test]
fn a_failed_fetch_is_a_question_retry_runs_it_again_and_run_without_passes_the_error() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    config(&w, json!({}));
    failing_fetch(&w);
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let asked = w.await_event("fetch.sh for extra review 1 failed");
    assert_eq!(
        asked.text,
        format!("fetch.sh for extra review 1 failed: {ERROR}")
    );
    let Some(Ask::TicketStart { options }) = asked.ask else {
        panic!("no Question: {:?}", asked.ask);
    };
    assert_eq!(options, ["retry", "run without it", "park"]);
    assert!(w.called("herdr agent start h-hx-1-extra-review").is_empty());
    o.answer("hx-1", "", Answer::Prompt("retry".to_string()));
    w.await_nth("fetch.sh for extra review 1 failed", 2);
    assert_eq!(w.called("env ").len(), 2);
    assert!(!o.ticket("hx-1").fetching);

    o.answer("hx-1", "", Answer::Prompt("run without it".to_string()));
    w.await_line("hx-1 running extra review 1 without its fetch");
    w.await_line("hx-1 PR #hx-1 opened");
    run.wait();
    let prompt = w.prompt("extra-review-1.md");
    let cache = o.cfg.repo.join(".orqadence-local/cache/security");
    assert!(
        prompt.contains(&format!("- Cache: {}\n", cache.display()))
            && prompt.contains(&format!("- Fetch: not run: {ERROR}\n")),
        "{prompt}"
    );
}

/// Park, picked on a failing fetch's Question, parks the Ticket with its
/// error; no Extra review starts.
#[test]
fn park_on_a_failed_fetch_parks_the_ticket_with_its_error() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    config(&w, json!({}));
    failing_fetch(&w);
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    w.await_event("fetch.sh for extra review 1 failed");
    o.answer("hx-1", "", Answer::Prompt("park".to_string()));
    run.wait();
    let reason = format!("fetch.sh for extra review 1 failed: {ERROR}");
    assert_eq!(o.ticket("hx-1").reason, reason);
    assert!(w.called("herdr agent start h-hx-1-extra-review").is_empty());
}

/// An Extra review a stopped run had started, its session still live in
/// its pane, is watched, not fetched for again: it may be reading the
/// cache. One whose pane is gone, or whose saved session does not resume
/// (its App changed), starts fresh and is fetched for, with the Cache.
#[test]
fn only_a_resumed_extra_review_is_not_fetched_for_again() {
    // (the pane is live, the saved session's App, fetches)
    for (live, app, fetched) in [(true, "", 0), (false, "", 1), (false, "claude", 1)] {
        let (w, o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
        config(&w, json!({}));
        let fetches = fetch_sh(&w);
        for file in ["implement.md", "review-1.md"] {
            write_file(&o.run_dir("hx-1").join(file), "STATUS: done\n");
        }
        o.update("hx-1", |ts| {
            ts.stage = "extra-review".to_string();
            ts.round = 1;
            ts.panes
                .insert("extra-review".to_string(), "1-9".to_string());
            if !app.is_empty() {
                let session = Session {
                    app: app.to_string(),
                    id: "s-1".to_string(),
                    ..Session::default()
                };
                ts.sessions.insert("extra-review".to_string(), session);
            }
        });
        if live {
            let mut w = w.lock();
            w.agents.insert("1-9".to_string(), "working".to_string());
            w.names
                .insert("h-hx-1-extra-review".to_string(), "1-9".to_string());
        }
        let o = Arc::new(o);
        let run = spawn_ticket(o.clone(), "hx-1");
        if live {
            wait_until("the live Extra review watched", || {
                !w.called("herdr agent get 1-9").is_empty()
            });
            write_file(
                &o.run_dir("hx-1").join("extra-review-1.md"),
                "STATUS: done\n",
            );
            w.lock()
                .agents
                .insert("1-9".to_string(), "idle".to_string());
        }
        w.await_line("hx-1 PR #hx-1 opened after 1 round");
        drop(run);
        assert_eq!(
            fetches.lock().unwrap().len(),
            fetched,
            "live: {live}, app: {app:?}"
        );
        let started = w.called("herdr agent start h-hx-1-extra-review").len();
        assert_eq!(started, fetched, "live: {live}, app: {app:?}");
        if !live {
            assert!(w.prompt("extra-review-1.md").contains("- Cache: "));
        }
    }
}

/// Away, a failing fetch parks the Ticket with a bd comment, like every
/// Question: no Extra review starts.
#[test]
fn away_a_failed_fetch_parks_the_ticket() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    config(&w, json!({}));
    failing_fetch(&w);
    o.cfg.away.store(true, Ordering::SeqCst);
    o.run_ticket("hx-1");

    w.await_line("hx-1 parked: asked you while away");
    let ts = o.ticket("hx-1");
    assert_eq!(
        (ts.status.as_str(), ts.reason.as_str()),
        (STATUS_PARKED, AWAY)
    );
    let comments = w.called("bd comments add hx-1 ");
    assert!(
        comments.len() == 1 && comments[0].contains(ERROR),
        "{comments:?}"
    );
    assert!(w.called("herdr agent start h-hx-1-extra-review").is_empty());
}

/// A label's review skill not installed goes under Not installed, named
/// for the review job, and its line is left out.
#[test]
fn an_extra_review_skill_not_installed_is_under_not_installed() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    labels(&w, json!({}));
    let _run = spawn_ticket(Arc::new(o), "hx-1");

    w.await_line("hx-1 PR #hx-1 opened");
    let prompt = w.prompt("extra-review-1.md");
    assert!(
        prompt.contains("- Not installed: orqa-sec-review (review)"),
        "{prompt}"
    );
    assert!(
        !prompt.contains("Use the orqa-sec-review skill"),
        "{prompt}"
    );
}

/// Position first: the Extra review runs in Round 1 only.
#[test]
fn position_first_runs_the_extra_review_in_round_one_only() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    config(&w, json!({"position": "first"}));
    w.session(|p| match (p.stage.as_str(), p.round) {
        ("verdict", 1) => (FIX.to_string(), "idle".to_string()),
        _ => succeed(p),
    });
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened after 2 rounds");
    assert_eq!(
        stages_run(&w),
        [
            "implement",
            "review",
            "extra-review",
            "debate",
            "fix",
            "review",
            "debate",
            "fix"
        ]
    );
}

/// A Ticket with no Area label, or one whose label carries no Extra
/// review, runs none.
#[test]
fn a_ticket_without_an_extra_review_runs_none() {
    for labels in [&["orqa:codex-review"][..], &["orqa:be"][..], &[][..]] {
        let (w, o) = new_world(vec![labelled_ticket(labels)]);
        let doc = json!({"labels": {
            "be": {"kind": "area", "extra_review": {"position": "first"}},
            "codex-review": {"kind": "modifier"},
        }});
        write_file(&w.repo.join(".orqadence/config.json"), &doc.to_string());
        o.run_ticket("hx-1");
        w.await_line("hx-1 PR #hx-1 opened");
        assert_eq!(
            stages_run(&w),
            ["implement", "review", "debate", "fix"],
            "{labels:?}"
        );
    }
}

/// Debate on: the Moderator gets the Extra review's result file as a second
/// Input, and a Verdict settling fewer Findings than both files hold is not
/// accepted.
#[test]
fn with_debate_on_the_moderator_gets_the_extra_review_file_and_settles_both() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    config(&w, json!({"debate": true}));
    // both reviews find one; the Verdict settles one
    w.session(|p| match p.stage.as_str() {
        "review" | "extra-review" => (FOUND.to_string(), "idle".to_string()),
        "verdict" => (SKIP.to_string(), "idle".to_string()),
        _ => succeed(p),
    });
    let o = Arc::new(o);
    let _run = spawn_ticket(o.clone(), "hx-1");

    w.await_line("hx-1 stuck in debate 1: Verdict settles 1 of the Review's 2 Findings");
    let extra = format!(
        "- Extra review file: {}\n",
        o.run_dir("hx-1").join("extra-review-1.md").display()
    );
    assert!(
        w.prompt("verdict-1.md").contains(&extra),
        "{}",
        w.prompt("verdict-1.md")
    );
}

/// Debate off: the Extra review's Findings skip the Debate and reach the
/// Fix as fix items marked not debated. They keep the Rounds going, and
/// Round 3's Fix, the one that opens the PR, still gets them.
#[test]
fn with_debate_off_the_findings_reach_the_fix_not_debated_and_keep_the_rounds_going() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    config(&w, json!({"debate": false}));
    w.session(|p| match p.stage.as_str() {
        "extra-review" => (FOUND.to_string(), "idle".to_string()),
        _ => succeed(p),
    });
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened after 3 rounds");
    let item = "- Fix items: \n  - [fix] (medium) a.go:1 — x | not debated | extra review\n";
    for round in 1..=3 {
        let fix = w.prompt(&format!("fix-{round}.md"));
        assert!(fix.contains(item), "{fix}");
        assert!(!w
            .prompt(&format!("verdict-{round}.md"))
            .contains("- Extra review file:"));
    }
    assert!(w.prompt("fix-3.md").contains("- Open PR: yes\n"));
    w.await_line("hx-1 debate 1 settled: 0 to fix, 0 skipped");
}

/// The Questions the Orchestrator put about a limit on the Review.
fn limit_questions(w: &World) -> Vec<(String, bool)> {
    w.events()
        .into_iter()
        .filter_map(|e| match e.ask {
            Some(Ask::Limited {
                app, extra_review, ..
            }) => Some((app, extra_review)),
            _ => None,
        })
        .collect()
}

/// The Extra review's App at its usage limit holds the Ticket as Implement's
/// would: no Question, no fallback, and it is not skipped.
#[test]
fn the_extra_reviews_app_limited_holds_the_ticket_with_no_question() {
    let (w, mut o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    config(&w, json!({"app": "codex"}));
    let clock = set_clock(
        &mut o.cfg,
        chrono::Local
            .with_ymd_and_hms(2026, 9, 25, 14, 0, 0)
            .unwrap(),
    );
    hits(&w, "hx-1", "extra-review", "idle", CODEX);
    let o = Arc::new(o);
    let run = spawn_ticket(o.clone(), "hx-1");

    w.await_line("hx-1 codex usage limit until 3:05pm: extra review 1 holds (pane 1-3)");
    assert!(!run.finished_within(std::time::Duration::from_millis(30)));
    assert_eq!(o.ticket("hx-1").limited, "codex");
    assert!(limit_questions(&w).is_empty(), "a Question was put");
    assert!(
        !w.lines().iter().any(|l| l.contains("stuck")),
        "a limit Woke"
    );
    assert!(w.called("herdr agent start h-hx-1-debate").is_empty());

    *clock.lock().unwrap() = chrono::Local
        .with_ymd_and_hms(2026, 9, 25, 15, 8, 0)
        .unwrap();
    w.await_line("hx-1 codex usage limit over: extra review 1 carries on");
    w.await_line("hx-1 PR #hx-1 opened");
}

/// A Round opened unreviewed skips the Extra review too: the limit Question
/// names it, RECENT says it was skipped, and the Fix's Unreviewed Input
/// says both were.
#[test]
fn an_unreviewed_round_skips_the_extra_review_and_the_fix_is_told() {
    let (w, mut o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    config(&w, json!({}));
    set_clock(
        &mut o.cfg,
        chrono::Local
            .with_ymd_and_hms(2026, 9, 25, 14, 0, 0)
            .unwrap(),
    );
    hits(&w, "hx-1", "review", "idle", CODEX);
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    wait_until("the Review's limit Question", || {
        !limit_questions(&w).is_empty()
    });
    assert_eq!(limit_questions(&w), [("codex".to_string(), true)]);
    o.review("codex", Review::Unreviewed);
    run.wait();

    w.await_line("hx-1 review 1 and debate 1 skipped: codex was limited until 3:05pm");
    w.await_line("hx-1 extra review 1 skipped: codex was limited until 3:05pm");
    w.await_line("hx-1 PR #hx-1 opened");
    assert!(w.called("herdr agent start h-hx-1-extra-review").is_empty());
    let fix = w.prompt("fix-1.md");
    assert!(
        fix.contains(
            "- Unreviewed: codex was limited until 3:05pm, the extra review skipped too\n"
        ),
        "{fix}"
    );
}

/// The limit Question's answer stands for every Ticket: one with an Extra
/// review holding for it while another asked puts the Question anew, by
/// the Ticket that asked, naming the Extra review.
#[test]
fn a_ticket_with_an_extra_review_joining_the_limit_question_names_it() {
    let two = BdTicket {
        labels: vec!["orqa:security".to_string()],
        ..BdTicket::new("hx-2")
    };
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1"), two]);
    config(&w, json!({}));
    set_clock(
        &mut o.cfg,
        chrono::Local
            .with_ymd_and_hms(2026, 9, 25, 14, 0, 0)
            .unwrap(),
    );
    write_file(&o.run_dir("hx-2").join("implement.md"), "STATUS: done\n");
    hits(&w, "hx-1", "review", "idle", CODEX);
    let o = Arc::new(o);
    let _one = spawn_ticket(o.clone(), "hx-1");
    wait_until("the Review's limit Question", || {
        !limit_questions(&w).is_empty()
    });
    assert_eq!(limit_questions(&w), [("codex".to_string(), false)]);

    let _two = spawn_ticket(o.clone(), "hx-2");
    wait_until("the Question put anew", || limit_questions(&w).len() == 2);
    assert_eq!(limit_questions(&w)[1], ("codex".to_string(), true));
    let asker: Vec<_> = w
        .events()
        .into_iter()
        .filter(|e| matches!(e.ask, Some(Ask::Limited { .. })))
        .filter_map(|e| e.ticket)
        .collect();
    assert_eq!(asker, ["hx-1", "hx-1"]);
    o.review("codex", Review::Unreviewed);
    w.await_line("hx-2 extra review 1 skipped: codex was limited until 3:05pm");
    assert_eq!(limit_questions(&w).len(), 2, "asked again after the answer");
}

/// Position before_pr: the last Round's Fix commits without opening the PR,
/// the Extra review runs once on the finished branch, its Debate follows,
/// and a final Fix, given the Verdict history with the final Debate's,
/// opens the PR.
#[test]
fn before_pr_the_last_fix_holds_the_pr_for_the_extra_review_its_debate_and_a_final_fix() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    config(&w, json!({"position": "before_pr", "debate": true}));
    w.session(|p| match (p.stage.as_str(), p.round) {
        ("extra-review", _) => (FOUND.to_string(), "idle".to_string()),
        ("verdict", 4) => (FIX.to_string(), "idle".to_string()),
        _ => succeed(p),
    });
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened after 1 round");
    assert_eq!(
        stages_run(&w),
        [
            "implement",
            "review",
            "debate",
            "fix",
            "extra-review",
            "debate",
            "fix"
        ]
    );
    w.await_line("hx-1 extra review before the PR found 1 finding");
    w.await_line("hx-1 debate final settled: 1 to fix, 0 skipped");
    let last = w.prompt("fix-1.md");
    assert!(last.contains("- Open PR: no\n"), "{last}");
    assert!(!last.contains("- Verdict history:"), "{last}");
    let debate = w.prompt("verdict-final.md");
    let file = format!(
        "- Review file: {}\n",
        o.run_dir("hx-1").join("extra-review-final.md").display()
    );
    assert!(debate.contains(&file), "{debate}");
    let fin = w.prompt("fix-final.md");
    assert!(fin.contains("- Open PR: yes\n"), "{fin}");
    assert!(
        fin.contains("- Fix items: \n  - [fix] (medium) a.go:1 — x | reason: agreed"),
        "{fin}"
    );
    assert!(fin.contains("verdict-1.md, "), "{fin}");
    assert!(fin.contains("verdict-final.md\n"), "{fin}");
    let files = format!(
        "- Extra review files: {}\n",
        o.run_dir("hx-1").join("extra-review-final.md").display()
    );
    assert!(fin.contains(&files), "{fin}");
    assert!(!last.contains("- Extra review files:"), "{last}");
}

/// No Findings: the final Fix still runs, with nothing to fix, and opens
/// the PR.
#[test]
fn before_pr_with_no_findings_the_final_fix_still_opens_the_pr() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    config(&w, json!({"position": "before_pr"}));
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened after 1 round");
    w.await_line("hx-1 extra review before the PR found 0 findings");
    let fin = w.prompt("fix-final.md");
    assert!(fin.contains("- Open PR: yes\n"), "{fin}");
    assert!(fin.contains("- Fix items: none\n"), "{fin}");
}

/// Debate off: no final Debate; its Findings go to the final Fix as fix
/// items marked not debated.
#[test]
fn before_pr_with_debate_off_the_final_fix_gets_the_findings_not_debated() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    config(&w, json!({"position": "before_pr", "debate": false}));
    w.session(|p| match p.stage.as_str() {
        "extra-review" => (FOUND.to_string(), "idle".to_string()),
        _ => succeed(p),
    });
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened after 1 round");
    assert_eq!(
        stages_run(&w),
        [
            "implement",
            "review",
            "debate",
            "fix",
            "extra-review",
            "fix"
        ]
    );
    let fin = w.prompt("fix-final.md");
    let item = "- Fix items: \n  - [fix] (medium) a.go:1 — x | not debated | extra review\n";
    assert!(fin.contains(item), "{fin}");
    assert!(fin.contains("- Open PR: yes\n"), "{fin}");
}

/// A run stopped after the final Extra review resumes at its Debate.
#[test]
fn before_pr_a_run_stopped_after_the_final_extra_review_resumes_at_its_debate() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    config(&w, json!({"position": "before_pr"}));
    let dir = o.run_dir("hx-1");
    for (file, text) in [
        ("implement.md", "STATUS: done\n"),
        ("review-1.md", "STATUS: done\n"),
        ("verdict-1.md", "STATUS: done\n"),
        ("fix-1.md", "STATUS: done\n"),
        ("extra-review-final.md", FOUND),
    ] {
        write_file(&dir.join(file), text);
    }
    w.session(|p| match p.stage.as_str() {
        "verdict" => (FIX.to_string(), "idle".to_string()),
        _ => succeed(p),
    });
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened after 1 round");
    assert_eq!(stages_run(&w), ["debate", "fix"]);
}

/// A Round opened unreviewed skips the Extra review before the PR: one Fix
/// opens the PR and says both were skipped.
#[test]
fn before_pr_an_unreviewed_last_round_goes_to_a_fix_that_opens_the_pr() {
    let (w, mut o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    config(&w, json!({"position": "before_pr"}));
    set_clock(
        &mut o.cfg,
        chrono::Local
            .with_ymd_and_hms(2026, 9, 25, 14, 0, 0)
            .unwrap(),
    );
    hits(&w, "hx-1", "review", "idle", CODEX);
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    wait_until("the Review's limit Question", || {
        !limit_questions(&w).is_empty()
    });
    o.review("codex", Review::Unreviewed);
    run.wait();

    w.await_line("hx-1 extra review before the PR skipped: codex was limited until 3:05pm");
    w.await_line("hx-1 PR #hx-1 opened after 1 round");
    assert_eq!(stages_run(&w), ["implement", "review", "fix"]);
    let fix = w.prompt("fix-1.md");
    assert!(fix.contains("- Open PR: yes\n"), "{fix}");
    assert!(
        fix.contains(
            "- Unreviewed: codex was limited until 3:05pm, the extra review skipped too\n"
        ),
        "{fix}"
    );
}

/// Labels that cannot be read where an Extra review is fetched for park
/// the Ticket: a fresh Extra review never starts without its fetch.
#[test]
fn labels_not_read_for_the_fetch_park_it() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    config(&w, json!({}));
    w.hook(|_, argv| match argv {
        ["bd", "show", ..] => Some(Err("dolt locked".to_string())),
        _ => None,
    });
    match o.fetch_inputs("hx-1", &EXTRA_REVIEW, "extra review 1") {
        Err(StageError::Parked(reason)) => assert!(reason.starts_with("Ticket labels not read")),
        other => panic!("not parked: {other:?}"),
    }
}

/// A stop that arrives while fetch.sh runs starts no Extra review.
#[test]
fn stop_during_the_fetch_starts_no_extra_review() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    config(&w, json!({}));
    fetch_sh(&w);
    let o = Arc::new(o);
    let stopper = o.clone();
    w.hook(move |_, argv| {
        if argv.first() == Some(&"env") {
            stopper.stop.store(true, Ordering::SeqCst);
        }
        None
    });
    let mut run = spawn_ticket(o.clone(), "hx-1");
    run.wait();
    assert_eq!(w.called("env ").len(), 1);
    assert!(w.called("herdr agent start h-hx-1-extra-review").is_empty());
}
