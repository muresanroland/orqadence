use super::herdr::PaneInfo;
use super::stage::{Answer, Ask, Orchestrator};
use super::state::{STATUS_PARKED, STATUS_PR_OPEN};
use super::world::{new_world, spawn_ticket, succeed, BdTicket, Prompt, World};
use super::write_file;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

pub(crate) fn stages_run(w: &World) -> Vec<String> {
    w.called("herdr agent start")
        .iter()
        .map(|call| {
            let name = call.split_whitespace().nth(3).unwrap();
            name[name.rfind('-').unwrap() + 1..].to_string()
        })
        .collect()
}

#[test]
fn implement_stage_runs_in_a_ticket_tab_and_reports_to_main() {
    let (w, o) = new_world(vec![BdTicket::new("hx-12")]);
    {
        let mut inner = w.lock();
        inner.tabs = vec!["w1:main-tab".to_string()];
        inner.panes = vec![PaneInfo {
            pane_id: "main".to_string(),
            tab_id: "w1:main-tab".to_string(),
        }];
    }

    o.run_ticket("hx-12");

    let got = w.called("bd worktree create");
    assert!(
        got.len() == 1 && got[0].ends_with(".orqadence-local/worktrees/hx-12 --branch hx-12"),
        "worktree calls = {got:?}"
    );
    let got = w.called("bd update hx-12");
    assert!(
        got.len() == 1 && got[0].contains("in_progress"),
        "ticket not marked in_progress: {got:?}"
    );
    let tab = w.called("herdr tab create");
    assert!(
        tab.len() == 1 && tab[0].contains("--label hx-12") && tab[0].contains("--no-focus"),
        "tab create = {tab:?}"
    );
    let start = &w.called("herdr agent start")[0];
    // Implement plans first (harness-kqe.13).
    for want in [
        "--kind claude".to_string(),
        "--permission-mode plan".to_string(),
        format!("--add-dir {}", o.run_dir("hx-12").display()),
    ] {
        assert!(start.contains(&want), "agent start lacks {want:?}: {start}");
    }
    // The panel is told where the session is and that it finished.
    w.await_line("hx-12 implement started: claude (pane 2-1)");
    w.await_line("hx-12 implemented");
    w.await_line("hx-12 review 1 started: codex (pane 2-2)");
    assert!(
        !w.lines().iter().any(|l| l.contains("prompted")),
        "'prompted' is for the log alone: {:?}",
        w.lines()
    );
    assert!(
        w.log()
            .contains(" hx-12 implement prompted, waiting for implement.md\n"),
        "log:\n{}",
        w.log()
    );
}

#[test]
fn clean_first_verdict_opens_pr_after_one_round() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);

    o.run_ticket("hx-1");

    assert_eq!(stages_run(&w), ["implement", "review", "debate", "fix"]);
    let ts = o.ticket("hx-1");
    assert!(
        ts.status == STATUS_PR_OPEN && ts.pr == "https://example.test/pr/hx-1",
        "state = {ts:?}"
    );
    w.await_line("hx-1 review 1 found 0 findings");
    w.await_line("hx-1 debate 1 settled: 0 to fix, 0 skipped");
    w.await_line("hx-1 fix 1 done");
    assert_eq!(
        w.await_line("hx-1 PR #hx-1 opened"),
        "hx-1 PR #hx-1 opened after 1 round",
        "the panel line carries no url"
    );
    assert!(
        w.log()
            .contains(" hx-1 PR #hx-1 opened after 1 round (https://example.test/pr/hx-1)\n"),
        "the log line adds the url:\n{}",
        w.log()
    );
    assert_eq!(
        w.called("herdr tab close").len(),
        1,
        "Ticket tab not closed once the PR opened"
    );
}

#[test]
fn review_runs_codex_in_the_run_directory_and_debate_gets_the_api_key() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    o.run_ticket("hx-1");

    let review = &w.called("herdr agent start")[1];
    assert!(
        review.contains("--kind codex") && review.contains("--sandbox workspace-write"),
        "review session = {review}"
    );
    let splits = w.called("herdr pane split");
    assert_eq!(
        splits.len(),
        3,
        "pane splits = {splits:?}, want one per Stage after Implement"
    );
    assert!(
        splits[0].contains(&format!("--cwd {}", o.run_dir("hx-1").display())),
        "Codex pane is not in the run directory: {}",
        splits[0]
    );
    assert!(
        splits[1].contains("--env TYPESAFE_API_KEY=sk-test") && !splits[2].contains("TYPESAFE"),
        "TYPESAFE_API_KEY must reach the Debate pane only: {:?}",
        &splits[1..]
    );
}

#[test]
fn ticket_that_keeps_producing_fix_items_gets_pr_after_exactly_three_rounds() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let fix_prompts: Arc<Mutex<Vec<Prompt>>> = Default::default();
    let seen = fix_prompts.clone();
    w.session(move |p| {
        match p.stage.as_str() {
            "verdict" => {
                return (
                    "STATUS: done\n- [fix] (high) a.go:1 — still broken | reason: agreed | settled: consensus\n".to_string(),
                    "idle".to_string(),
                )
            }
            "fix" => seen.lock().unwrap().push(p.clone()),
            _ => {}
        }
        succeed(p)
    });

    o.run_ticket("hx-1");

    let want = [
        "implement",
        "review",
        "debate",
        "fix",
        "review",
        "debate",
        "fix",
        "review",
        "debate",
        "fix",
    ];
    assert_eq!(stages_run(&w), want);
    let fix_prompts = fix_prompts.lock().unwrap();
    assert!(
        fix_prompts.len() == 3
            && !fix_prompts[0].open_pr
            && !fix_prompts[1].open_pr
            && fix_prompts[2].open_pr,
        "only the round 3 Fix session may open the PR: {fix_prompts:?}"
    );
    for round in 1..=3 {
        let file = format!("verdict-{round}.md");
        assert!(
            fix_prompts[2].text.contains(&file),
            "last Fix prompt lacks {file} for the PR's Verdict history"
        );
    }
    w.await_line("hx-1 PR #hx-1 opened after 3 rounds");
}

#[test]
fn open_pr_prunes_build_scratch_and_keeps_evidence() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    // The Review Stage compiles the branch, and the run directory is the only
    // place its sandbox may write, so its build cache lands there.
    let run_dir = o.run_dir("hx-1");
    let dir = run_dir.clone();
    w.session(move |p| {
        if p.stage == "review" {
            write_file(&dir.join(".review-cache/ab/obj-a"), "go object data");
            write_file(&dir.join("check-testharness"), "a compiled test binary");
            write_file(&dir.join("diff-1.patch"), "the diff it reviewed");
        }
        succeed(p)
    });

    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened after 1 round");
    for gone in [".review-cache", "check-testharness"] {
        assert!(
            !run_dir.join(gone).exists(),
            "{gone} still in the run directory after the PR opened"
        );
    }
    for kept in [
        "implement.md",
        "review-1.md",
        "verdict-1.md",
        "fix-1.md",
        "diff-1.patch",
    ] {
        assert!(run_dir.join(kept).exists(), "evidence pruned: {kept}");
    }
}

/// A Ticket runs the Stage skill committed on its base, as its worktree
/// checks it out, not the checkout's.
#[test]
fn implement_runs_the_stage_skill_of_its_worktree() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    // the base's stage-implement, once the worktree checks it out
    w.hook(|dir, argv| {
        if argv.starts_with(&["git", "pull"]) {
            let skill = dir.join(".orqadence/skills/stage-implement/SKILL.md");
            write_file(&skill, "The base's Implement: do the Ticket.\n");
        }
        None
    });
    o.run_ticket("hx-1");

    let implement = w.prompt("implement.md");
    assert!(
        implement.contains("The base's Implement: do the Ticket."),
        "{implement}"
    );
    assert!(!implement.contains("# Implement Stage"), "{implement}");
}

/// The absolute links an Orqadence from before ADR 0006 made in a worktree
/// to the checkout's skills go; the worktree's own relative ones stay.
#[test]
fn a_worktree_loses_the_links_to_the_checkouts_skills() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let checkout = w.repo.join(".orqadence/skills/stage-implement");
    w.hook(move |dir, argv| {
        if argv.starts_with(&["git", "pull"]) {
            std::fs::create_dir_all(dir.join(".claude/skills")).unwrap();
            std::fs::create_dir_all(dir.join(".agents/skills")).unwrap();
            let old = dir.join(".claude/skills/stage-implement");
            let _ = std::fs::remove_file(&old);
            std::os::unix::fs::symlink(&checkout, old).unwrap();
            let own = dir.join(".agents/skills/own");
            std::os::unix::fs::symlink("../../.orqadence/skills/own", own).unwrap();
        }
        None
    });
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened");
    let tree = o.worktree("hx-1");
    let old = tree.join(".claude/skills/stage-implement");
    assert!(
        std::fs::read_link(&old).map_or(true, |to| !to.is_absolute()),
        "{old:?} still links the checkout"
    );
    assert!(std::fs::symlink_metadata(tree.join(".agents/skills/own")).is_ok());
}

/// An old link that cannot be removed parks the Ticket before any Stage:
/// left, the Stage would run the checkout's skill.
#[test]
fn an_old_link_not_removed_parks_the_ticket() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |dir: &std::path::Path, mode| {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode)).unwrap()
    };
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let checkout = w.repo.join(".orqadence/skills/stage-implement");
    w.hook(move |dir, argv| {
        if argv.starts_with(&["git", "pull"]) {
            let links = dir.join(".claude/skills");
            std::fs::create_dir_all(&links).unwrap();
            std::os::unix::fs::symlink(&checkout, links.join("stage-implement")).unwrap();
            mode(&links, 0o555);
        }
        None
    });
    o.run_ticket("hx-1");
    mode(&o.worktree("hx-1").join(".claude/skills"), 0o755);

    let ts = o.ticket("hx-1");
    assert_eq!(ts.status, STATUS_PARKED);
    assert!(ts.reason.starts_with("old link"), "{}", ts.reason);
    assert!(w.called("herdr agent start").is_empty(), "a Stage started");
}

/// What the Ticket-start Question says of tdd, test-first's pick.
const UNMERGED: &str = "tdd, picked for test-first, is not on this Ticket's base branch: \
                        added in /config and not yet merged";

/// Implement's Not installed input, "" when it has none.
fn not_installed(w: &World) -> String {
    let implement = w.prompt("implement.md");
    let line = implement
        .lines()
        .find(|l| l.starts_with("- Not installed: "));
    line.unwrap_or_default().to_string()
}

/// How many Events put a Question.
fn asked(w: &World) -> usize {
    w.events().iter().filter(|e| e.ask.is_some()).count()
}

/// Answers the Ticket-start Question about tdd with option `n`, from 0.
fn answer_tdd(w: &World, o: &Orchestrator, nth: usize, n: usize) {
    let asked = w.await_nth("picked for test-first", nth);
    let Some(Ask::TicketStart { options }) = asked.ask else {
        panic!("no Ticket-start Question: {:?}", asked.ask);
    };
    o.answer("hx-1", "", Answer::Prompt(options[n].clone()));
}

/// A pick the checkout's Skill manifest records but the base lacks was
/// added in /config and not yet merged: a Question before any Stage. Run
/// without it leaves the job's line out, said under Not installed.
#[test]
fn a_pick_not_merged_on_the_base_is_a_question_when_the_ticket_starts() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.unmerged("test-first", "tdd");
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    assert_eq!(w.await_event("picked for test-first").text, UNMERGED);
    assert!(w.called("herdr agent start").is_empty(), "a Stage started");
    answer_tdd(&w, &o, 1, 1);
    run.wait();

    w.await_line("hx-1 running without tdd: the test-first line is left out");
    w.await_line("hx-1 PR #hx-1 opened");
    assert!(
        not_installed(&w).contains("tdd (test-first)"),
        "{}",
        not_installed(&w)
    );
}

/// Run without it leaves the line out even when a personal skill has the
/// pick's name: the pick is the manifest's, which the base lacks.
#[test]
fn run_without_a_pick_not_merged_leaves_out_a_personal_skill_of_its_name() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.unmerged("test-first", "tdd");
    for dir in [".claude/skills", ".agents/skills"] {
        write_file(
            &w.home.join(dir).join("tdd/SKILL.md"),
            "---\nname: tdd\n---\n",
        );
    }
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    answer_tdd(&w, &o, 1, 1);
    run.wait();

    w.await_line("hx-1 PR #hx-1 opened");
    assert!(!w.prompt("implement.md").contains("Use the tdd skill"));
    assert!(not_installed(&w).contains("tdd (test-first)"));
}

/// Park parks the Ticket with the reason, and /continue asks again until
/// the skill is merged: then the branch, still the base's, is brought up
/// to it, and the job's line names it.
#[test]
fn parked_for_a_pick_not_merged_a_ticket_asks_again_until_it_is() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.unmerged("test-first", "tdd");
    let o = Arc::new(o);
    let reason = "tdd not merged: commit and merge it, then /continue @hx-1";
    let mut runs = Vec::new(); // kept to the end: dropping one stops the run
    for nth in 1..=2 {
        runs.push(spawn_ticket(o.clone(), "hx-1")); // /continue @hx-1, the second time
        answer_tdd(&w, &o, nth, 0);
        runs.last_mut().unwrap().wait();
        let ts = o.ticket("hx-1");
        assert_eq!(
            (ts.status.as_str(), ts.reason.as_str()),
            (STATUS_PARKED, reason)
        );
    }
    w.await_line(&format!("hx-1 parked: {reason}"));
    assert!(w.called("herdr agent start").is_empty(), "a Stage started");

    w.merge();
    runs.push(spawn_ticket(o.clone(), "hx-1"));
    runs.last_mut().unwrap().wait();
    w.await_line("hx-1 PR #hx-1 opened");
    assert_eq!(asked(&w), 2, "asked again once merged");
    assert!(w.prompt("implement.md").contains("Use the tdd skill"));
}

/// Away, the Ticket-start Question parks the Ticket with a bd comment, as a
/// Stage's own question does: nothing is asked and no Stage starts, until
/// /continue asks it.
#[test]
fn away_a_pick_not_merged_parks_the_ticket_without_asking() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.unmerged("test-first", "tdd");
    o.cfg.away.store(true, Ordering::SeqCst);
    o.run_ticket("hx-1");

    w.await_line("hx-1 parked: asked you while away");
    assert_eq!(o.ticket("hx-1").status, STATUS_PARKED);
    let comments = w.called("bd comments add hx-1 ");
    assert!(
        comments.len() == 1
            && comments[0].contains("/continue @hx-1")
            && comments[0].contains(UNMERGED)
            && comments[0].contains("- run without it: the test-first line is left out"),
        "bd comments = {comments:?}"
    );
    assert_eq!(asked(&w), 0, "Away still asked");
    assert!(w.called("herdr agent start").is_empty(), "a Stage started");

    o.cfg.away.store(false, Ordering::SeqCst);
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    answer_tdd(&w, &o, 1, 1);
    run.wait();
    w.await_line("hx-1 PR #hx-1 opened");
}

/// A pick merged on the base is in the worktree: nothing is asked, and its
/// line names it.
#[test]
fn a_pick_merged_on_the_base_asks_nothing() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.picked("test-first", "tdd");
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened");
    assert_eq!(asked(&w), 0, "a Question was put");
    assert!(w.prompt("implement.md").contains("Use the tdd skill"));
    assert!(!not_installed(&w).contains("tdd"), "{}", not_installed(&w));
}

/// Once a Stage has run, the branch holds the Ticket's work and is no
/// longer the base's: a continued Ticket is not asked, and a pick missing
/// there is not installed, as any.
#[test]
fn a_ticket_past_its_start_is_not_asked() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.unmerged("test-first", "tdd");
    o.update("hx-1", |ts| ts.stage = "implement".to_string());
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened");
    assert_eq!(asked(&w), 0, "a Question was put");
    assert!(not_installed(&w).contains("tdd (test-first)"));
}

/// Parked for a pick not merged, a Ticket whose pull then fails parks with
/// the pull's error: it neither asks again nor starts a Stage on a stale base.
#[test]
fn a_failed_pull_on_continue_parks_the_ticket() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.unmerged("test-first", "tdd");
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    answer_tdd(&w, &o, 1, 0);
    run.wait();

    w.merge();
    w.fail_once("git pull", "network down");
    o.run_ticket("hx-1");

    let ts = o.ticket("hx-1");
    assert_eq!(ts.status, STATUS_PARKED);
    assert!(
        ts.reason.starts_with("branch not brought up") && ts.reason.ends_with("network down"),
        "{}",
        ts.reason
    );
    assert_eq!(asked(&w), 1, "asked again on a stale base");
    assert!(w.called("herdr agent start").is_empty(), "a Stage started");
}

/// A Review or a Debate that leaves the worktree dirty, or commits, is put
/// back to the HEAD recorded before it through git, and said on RECENT.
#[test]
fn a_review_or_debate_that_dirties_or_commits_the_worktree_is_restored_and_says_so() {
    // The world names a Stage by its result file: the Debate's is verdict.
    for (stage, prompt) in [("review", "review"), ("debate", "verdict")] {
        for (head, status) in [("a11ce", " M src/lib.rs\n"), ("c0ffee", "")] {
            let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
            // The worktree as git reports it: HEAD and the status's output.
            let tree = Arc::new(Mutex::new(("a11ce".to_string(), String::new())));
            let changed = tree.clone();
            w.session(move |p| {
                if p.stage == prompt {
                    *changed.lock().unwrap() = (head.to_string(), status.to_string());
                }
                succeed(p)
            });
            let git = tree.clone();
            w.hook(move |_, argv| {
                let mut tree = git.lock().unwrap();
                match argv {
                    ["git", "rev-parse", "HEAD"] => Some(Ok(format!("{}\n", tree.0))),
                    ["git", "status", "--porcelain"] => Some(Ok(tree.1.clone())),
                    ["git", "reset", "--hard", to] => {
                        *tree = (to.to_string(), String::new());
                        Some(Ok(String::new()))
                    }
                    _ => None,
                }
            });
            o.run_ticket("hx-1");

            w.await_line(&format!("hx-1 {stage} 1 changed the worktree: restored"));
            let case = format!("{stage} {head} {status:?}");
            assert_eq!(w.called("git reset --hard a11ce").len(), 1, "{case}");
            assert_eq!(w.called("git clean -fd").len(), 1, "{case}");
            w.await_line("hx-1 PR #hx-1 opened");
        }
    }
}

/// A Review parked after it dirtied the worktree has its session ended and
/// the tree put back at once, since the Ticket may never continue. The
/// parked tree is then the user's: an edit made while parked survives the
/// continued Review.
#[test]
fn a_review_parked_after_dirtying_the_worktree_is_restored_at_once() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let status = Arc::new(Mutex::new(String::new()));
    let changed = status.clone();
    w.session(move |p| {
        if p.stage != "review" {
            return succeed(p);
        }
        *changed.lock().unwrap() = " M src/lib.rs\n".to_string();
        (String::new(), "blocked".to_string())
    });
    let git = status.clone();
    w.hook(move |_, argv| match argv {
        ["git", "status", "--porcelain"] => Some(Ok(git.lock().unwrap().clone())),
        ["git", "reset", "--hard", _] => {
            git.lock().unwrap().clear();
            Some(Ok(String::new()))
        }
        _ => None,
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    w.await_line("hx-1 waiting at a prompt in review 1");
    o.command("park-hx-1");
    run.wait();

    w.await_line("hx-1 review 1 changed the worktree: restored");
    w.await_line("hx-1 parked: by you at review 1");
    assert_eq!(w.called("git clean -fd").len(), 1);
    assert!(!o.run_dir("hx-1").join("before-review-1.json").exists());
    // no session is left to write to the tree once it is put back
    assert_eq!(w.called("herdr pane close").len(), 1);
    assert!(!o.ticket("hx-1").panes.contains_key("review"));

    // the user edits the parked tree, then continues with a fresh Review
    *status.lock().unwrap() = " M notes.md\n".to_string();
    w.session(succeed);
    o.run_ticket("hx-1");
    w.await_line("hx-1 PR #hx-1 opened");
    assert_eq!(w.called("git clean -fd").len(), 1, "user edit reset");
}

/// A Review parked after changing a worktree it cannot put back says so in
/// the park reason, and keeps its snapshot for the continued Review.
#[test]
fn a_parked_review_whose_tree_is_not_restored_says_so() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let tree = Arc::new(Mutex::new(" M src/lib.rs\n"));
    let reviewed = tree.clone();
    w.session(move |p| {
        if p.stage != "review" {
            return succeed(p);
        }
        *reviewed.lock().unwrap() = " M src/lib.rs\n M src/main.rs\n";
        (String::new(), "blocked".to_string())
    });
    w.hook(move |_, argv| match argv {
        ["git", "status", "--porcelain"] => Some(Ok(tree.lock().unwrap().to_string())),
        _ => None,
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    w.await_line("hx-1 waiting at a prompt in review 1");
    o.command("park-hx-1");
    run.wait();

    w.await_line(
        "hx-1 parked: by you at review 1; \
         review 1 changed a worktree already dirty before it, not restored",
    );
    assert!(o.run_dir("hx-1").join("before-review-1.json").exists());
}

/// A Ticket parked because its Review changed a worktree already dirty is
/// guarded again when continued, though its Review is done: it parks again,
/// with no Debate, until the tree is put back by hand.
#[test]
fn a_continued_ticket_is_guarded_against_its_done_review() {
    let parked = "hx-1 parked: review 1 changed a worktree already dirty before it, not restored";
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let tree = Arc::new(Mutex::new(" M src/lib.rs\n"));
    let reviewed = tree.clone();
    w.session(move |p| {
        if p.stage == "review" {
            *reviewed.lock().unwrap() = " M src/lib.rs\n M src/main.rs\n";
        }
        succeed(p)
    });
    let git = tree.clone();
    w.hook(move |_, argv| match argv {
        ["git", "status", "--porcelain"] => Some(Ok(git.lock().unwrap().to_string())),
        _ => None,
    });

    for times in 1..=2 {
        o.run_ticket("hx-1");
        let lines = w.lines();
        assert_eq!(lines.iter().filter(|l| l.contains(parked)).count(), times);
        assert_eq!(stages_run(&w), ["implement", "review"]);
    }
    *tree.lock().unwrap() = " M src/lib.rs\n";
    o.run_ticket("hx-1");
    w.await_line("hx-1 PR #hx-1 opened");
    assert_eq!(stages_run(&w), ["implement", "review", "debate", "fix"]);
}

#[test]
fn a_clean_review_changes_nothing() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened");
    assert!(
        !w.called("git rev-parse HEAD").is_empty(),
        "HEAD never recorded"
    );
    assert!(
        w.called("git reset").is_empty() && w.called("git clean").is_empty(),
        "{}",
        w.calls().join("\n")
    );
    assert!(!w.lines().iter().any(|l| l.contains("changed the worktree")));
}

/// A resumed run skips a Review already done, and its guard with it: the
/// tree may hold a later Stage's work.
#[test]
fn a_review_already_done_is_not_guarded_again() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    for name in ["implement.md", "review-1.md"] {
        write_file(&o.run_dir("hx-1").join(name), "STATUS: done\n");
    }
    w.hook(|_, argv| match argv {
        ["git", "status", "--porcelain"] => Some(Ok(" M src/lib.rs\n".to_string())),
        _ => None,
    });
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened");
    assert!(w.called("git reset").is_empty(), "{}", w.calls().join("\n"));
}

/// A worktree already dirty before the Review holds work that is not the
/// Review's: it is never reset. Left as it was, the Pipeline goes on; changed
/// by the Review, even only in the content of a file already changed or
/// untracked, the Ticket parks with the work kept.
#[test]
fn a_worktree_dirty_before_the_review_is_never_reset() {
    let parked = "hx-1 parked: review 1 changed a worktree already dirty before it, not restored";
    // The tree as git reports it: the status, `git diff HEAD`, the one
    // untracked file's path and its hash.
    let dirty = (
        " M src/lib.rs\n?? notes/\n",
        "+one\n",
        "notes/a.txt\0",
        "e69de29\n",
    );
    for (after, want) in [
        (dirty, "hx-1 PR #hx-1 opened"),
        ((" M src/lib.rs\n", dirty.1, dirty.2, dirty.3), parked),
        ((dirty.0, "+two\n", dirty.2, dirty.3), parked),
        ((dirty.0, dirty.1, "notes/b.txt\0", dirty.3), parked),
        ((dirty.0, dirty.1, dirty.2, "d00491f\n"), parked),
    ] {
        let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
        let tree = Arc::new(Mutex::new(dirty));
        let reviewed = tree.clone();
        w.session(move |p| {
            if p.stage == "review" {
                *reviewed.lock().unwrap() = after;
            }
            succeed(p)
        });
        w.hook(move |_, argv| {
            let tree = tree.lock().unwrap();
            match argv {
                ["git", "status", "--porcelain"] => Some(Ok(tree.0.to_string())),
                ["git", "diff", "HEAD", "--binary"] => Some(Ok(tree.1.to_string())),
                ["git", "ls-files", "--others", ..] => Some(Ok(tree.2.to_string())),
                ["git", "hash-object", "--", _] => Some(Ok(tree.3.to_string())),
                _ => None,
            }
        });
        o.run_ticket("hx-1");

        w.await_line(want);
        assert!(
            w.called("git reset").is_empty() && w.called("git clean").is_empty(),
            "{}",
            w.calls().join("\n")
        );
    }
}
