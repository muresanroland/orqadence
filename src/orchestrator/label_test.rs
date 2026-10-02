//! Ticket labels on the code-editing Stages: the label's skills and
//! guidance as Inputs, the Epic context in the Ticket file, and a label
//! skill missing on the base as a Question at Ticket start. And the GitHub
//! labels a Ticket's PR gets as it opens: orqa:human-merge, orqa:no-review
//! and the Ticket's own; and orqa:no-review on a Ticket, which skips its
//! Review.

use super::stage::{Answer, Ask};
use super::state::{load_state, STATUS_PR_OPEN};
use super::world::{new_world, spawn_epic, spawn_ticket, BdTicket, World};
use super::write_file;
use crate::skills::SKILLS;
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// hx-1 with the bd labels given.
fn labelled_ticket(labels: &[&str]) -> BdTicket {
    BdTicket {
        labels: labels.iter().map(|l| l.to_string()).collect(),
        ..BdTicket::new("hx-1")
    }
}

/// config.json with orqa:db, an Area label with the skills and guidance
/// given, and orqa:codex-review, a Modifier.
fn config(w: &World, skills: &[&str], guidance: &str) {
    let doc = json!({"labels": {
        "db": {"kind": "area", "skills": skills, "guidance": guidance},
        "codex-review": {"kind": "modifier"},
    }});
    write_file(&w.repo.join(".orqadence/config.json"), &doc.to_string());
}

/// The Label Inputs of the prompt writing `file`, in order.
fn label_inputs(w: &World, file: &str) -> Vec<String> {
    w.prompt(file)
        .lines()
        .filter(|l| l.starts_with("- Label"))
        .map(String::from)
        .collect()
}

/// A Ticket's labels reach the code-editing Stages, Implement, Fix, Rebase
/// and Address PR comments, as Inputs: the labels, Area first, the
/// entries' installed skills and their guidance, with the Ticket file. The
/// Review and the Debate get none of them.
#[test]
fn the_code_editing_stages_get_the_labels_skills_and_guidance_and_the_review_none() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:codex-review", "orqa:db"])]);
    config(&w, &["orqa-db-skill"], "Migrations are reversible.");
    w.installed("orqa-db-skill");
    w.lock().prs.insert(
        "https://example.test/pr/hx-1".to_string(),
        r#"{"state":"OPEN","mergeable":"CONFLICTING"}"#.to_string(),
    );
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");
    w.await_line("hx-1 PR #hx-1 conflicts with main, /rebase resolves it");
    o.command("rebase-hx-1");
    w.await_line("hx-1 rebased PR #hx-1");
    o.command("address-pr-comments-hx-1");
    w.await_line("hx-1 addressed PR #hx-1");
    o.stop();
    run.wait();
    o.wait_in_flight();

    let want = [
        "- Label: orqa:db, orqa:codex-review",
        "- Label skills: orqa-db-skill",
        "- Label guidance: Migrations are reversible.",
    ];
    let ticket = format!(
        "- Ticket file: {}",
        o.run_dir("hx-1").join("ticket.md").display()
    );
    for file in [
        "implement.md",
        "fix-1.md",
        "rebase.md",
        "address-pr-comments.md",
    ] {
        assert_eq!(label_inputs(&w, file), want, "{file}");
        assert!(w.prompt(file).contains(&ticket), "{file}");
    }
    for file in ["review-1.md", "verdict-1.md"] {
        assert_eq!(label_inputs(&w, file), Vec::<String>::new(), "{file}");
        assert!(!w.prompt(file).contains("Ticket file"), "{file}");
    }
    assert!(!not_installed(&w).contains("orqa-db-skill"));
}

/// Implement's Not installed input, "" when it has none.
fn not_installed(w: &World) -> String {
    let implement = w.prompt("implement.md");
    let line = implement
        .lines()
        .find(|l| l.starts_with("- Not installed: "));
    line.unwrap_or_default().to_string()
}

/// A label skill the Stage's App cannot load, or that is missing, goes
/// under Not installed, named with its label, and not under Label skills.
#[test]
fn a_label_skill_not_installed_is_under_not_installed() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:db"])]);
    config(&w, &["orqa-db-skill"], "");
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened");
    assert_eq!(label_inputs(&w, "implement.md"), ["- Label: orqa:db"]);
    assert!(
        not_installed(&w).contains("orqa-db-skill (orqa:db)"),
        "{}",
        not_installed(&w)
    );
}

/// A personal ~/.claude/skills copy of a committed label skill shadows it:
/// a Question before any Stage, as a pick's is. Parked, no Stage starts.
#[test]
fn a_personal_copy_of_a_label_skill_is_a_question_at_ticket_start() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:db"])]);
    config(&w, &["orqa-db-skill"], "");
    w.installed("orqa-db-skill");
    write_file(
        &w.home.join(".claude/skills/orqa-db-skill/SKILL.md"),
        "yours",
    );
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let asked = w.await_event("shadows the committed");
    assert_eq!(
        asked.text,
        "your ~/.claude/skills/orqa-db-skill shadows the committed orqa-db-skill: claude runs yours"
    );
    assert!(w.called("herdr agent start").is_empty(), "a Stage started");
    let Some(Ask::TicketStart { options }) = asked.ask else {
        panic!("no Ticket-start Question: {:?}", asked.ask);
    };
    o.answer("hx-1", "", Answer::Prompt(options[1].clone()));
    run.wait();
    assert!(w.called("herdr agent start").is_empty(), "a Stage started");
}

/// The Ticket file is written as each code-editing Stage starts, the parent
/// Epic's description under ## Epic context, and Fix gets it as an Input
/// too. A Ticket without a parent has no such heading.
#[test]
fn the_ticket_file_ends_with_the_epics_description_and_fix_gets_it() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.lock().epic_description = "Both sides expect a JSON API.".to_string();
    w.hook(|_, argv| (argv == ["bd", "show", "hx-1"]).then(|| Ok("hx-1 · the Ticket\n".into())));
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened");
    let ticket = o.run_dir("hx-1").join("ticket.md");
    assert_eq!(
        std::fs::read_to_string(&ticket).unwrap(),
        "hx-1 · the Ticket\n\n## Epic context\n\nBoth sides expect a JSON API.\n"
    );
    let input = format!("\n- Ticket file: {}\n", ticket.display());
    assert!(
        w.prompt("fix-1.md").contains(&input),
        "{}",
        w.prompt("fix-1.md")
    );
    assert!(!w.prompt("review-1.md").contains(&input));

    let alone = BdTicket {
        no_epic: true,
        ..BdTicket::new("hx-1")
    };
    let (w, o) = new_world(vec![alone]);
    w.lock().epic_description = "Not this Ticket's.".to_string();
    w.hook(|_, argv| (argv == ["bd", "show", "hx-1"]).then(|| Ok("hx-1 · the Ticket\n".into())));
    o.run_ticket("hx-1");
    w.await_line("hx-1 PR #hx-1 opened");
    let ticket = o.run_dir("hx-1").join("ticket.md");
    assert_eq!(
        std::fs::read_to_string(&ticket).unwrap(),
        "hx-1 · the Ticket\n"
    );
}

/// A label skill, or a label's Extra review skill, the checkout's Skill
/// manifest records but the Ticket's base lacks is a Question before any
/// Stage. Run without it starts Implement without it: under Not installed,
/// not under Label skills, for this Ticket's run.
#[test]
fn a_label_skill_missing_from_the_base_is_a_question_at_ticket_start() {
    for (skills, extra) in [(vec!["orqa-db-skill"], ""), (vec![], "orqa-db-skill")] {
        let (w, o) = new_world(vec![labelled_ticket(&["orqa:db"])]);
        let doc = json!({"labels": {"db": {"kind": "area", "skills": skills,
            "extra_review": {"skill": extra}}}});
        write_file(&w.repo.join(".orqadence/config.json"), &doc.to_string());
        w.unmerged_skill("orqa-db-skill");
        let o = Arc::new(o);
        let mut run = spawn_ticket(o.clone(), "hx-1");

        let text = "orqa-db-skill is not committed on hx-1's base: commit and merge it first, \
                    or run without it";
        let asked = w.await_event("is not committed");
        assert_eq!(asked.text, text);
        assert!(w.called("herdr agent start").is_empty(), "a Stage started");
        let Some(Ask::TicketStart { options }) = asked.ask else {
            panic!("no Ticket-start Question: {:?}", asked.ask);
        };
        assert_eq!(
            options,
            [
                "park: commit and merge orqa-db-skill, then /continue @hx-1",
                "run without it"
            ]
        );
        o.answer("hx-1", "", Answer::Prompt(options[1].clone()));
        run.wait();

        w.await_line("hx-1 running without orqa-db-skill");
        w.await_line("hx-1 PR #hx-1 opened");
        assert_eq!(label_inputs(&w, "implement.md"), ["- Label: orqa:db"]);
        assert_eq!(
            not_installed(&w).contains("orqa-db-skill (orqa:db)"),
            extra.is_empty(),
            "{}",
            not_installed(&w)
        );
    }
}

/// Each code-editing Stage skill says to load the Label skills and follow
/// the Label guidance, and that the Epic context is context, not scope.
#[test]
fn the_code_editing_stage_skills_load_the_label_skills_and_bound_the_epic_context() {
    let line = "Load every skill under **Label skills** by name in this session, and follow \
                **Label guidance**.";
    for name in [
        "orqa-stage-implement",
        "orqa-stage-fix",
        "orqa-stage-rebase",
        "orqa-stage-address-pr-comments",
    ] {
        let skill = SKILLS.iter().find(|(n, _)| *n == name).unwrap().1;
        assert!(skill.contains(line), "{name} lacks the label line");
        assert!(
            skill.contains("**Ticket file**") && skill.contains("context, not scope"),
            "{name} does not bound the Epic context"
        );
    }
}

/// The PR's changed files, as git lists them against the base.
const DIFF: &str = "git diff --no-renames --name-only -z origin/main...HEAD";

/// The Ticket's branch changed these paths.
fn changed(w: &World, paths: &'static [&'static str]) {
    w.hook(move |_, argv| (argv.join(" ") == DIFF).then(|| Ok(paths.join("\0") + "\0")));
}

/// Markdown anywhere, and a file of any kind under each skills folder.
/// docs/café.md, which git quotes unless -z.
const MARKDOWN_AND_SKILLS: [&str; 7] = [
    "README.md",
    "docs/café.md",
    "docs/adr/0007.md",
    "skills/orqa-x/fetch.sh",
    ".orqadence/skills/orqa-x/SKILL.md",
    ".claude/skills/orqa-x/run.sh",
    ".agents/skills/orqa-x/check.py",
];

const PR: &str = "https://example.test/pr/hx-1";

/// A PR whose every changed file is Markdown or a skill is a No-review
/// pull request: the Orchestrator labels it orqa:no-review as it opens,
/// and the Ticket's saved state keeps it.
#[test]
fn a_pr_of_markdown_and_skills_alone_gets_orqa_no_review() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    changed(&w, &MARKDOWN_AND_SKILLS);

    o.run_ticket("hx-1");

    assert_eq!(
        w.called("gh pr edit"),
        [format!("gh pr edit {PR} --add-label orqa:no-review")]
    );
    assert_eq!(w.called("gh label create"), Vec::<String>::new());
    let ts = o.ticket("hx-1");
    assert!(ts.no_review && !ts.human_merge, "state = {ts:?}");
    let saved = load_state(&w.repo).unwrap().tickets["hx-1"].clone();
    assert!(
        saved.no_review && saved.status == STATUS_PR_OPEN,
        "{saved:?}"
    );
}

/// One file that is neither Markdown nor a skill's, and the PR is reviewed:
/// no label, and nothing created on GitHub. A skills folder counts from
/// the repo's root alone.
#[test]
fn a_pr_touching_src_gets_no_label() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    changed(&w, &["docs/adr/0007.md", "src/skills/run.sh"]);

    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened");
    assert_eq!(w.called("gh pr edit"), Vec::<String>::new());
    assert_eq!(w.called("gh label create"), Vec::<String>::new());
    let ts = o.ticket("hx-1");
    assert!(!ts.no_review && !ts.human_merge, "state = {ts:?}");
}

/// Human-merge and No-review are each on their own: a security Ticket's
/// PR of Markdown and skills alone is labelled both, then orqa:security,
/// so the bots skip it and a human merges it.
#[test]
fn a_security_tickets_pr_of_markdown_alone_gets_orqa_human_merge_and_no_review() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:security"])]);
    let doc = json!({"labels": {"security": {"kind": "area", "human_merge": true}}});
    write_file(&w.repo.join(".orqadence/config.json"), &doc.to_string());
    changed(&w, &MARKDOWN_AND_SKILLS);

    o.run_ticket("hx-1");

    assert_eq!(
        w.called("gh pr edit"),
        [
            format!("gh pr edit {PR} --add-label orqa:human-merge"),
            format!("gh pr edit {PR} --add-label orqa:no-review"),
            format!("gh pr edit {PR} --add-label orqa:security"),
        ]
    );
    let ts = o.ticket("hx-1");
    assert!(ts.human_merge && ts.no_review, "state = {ts:?}");
}

/// A PR carries its Ticket's orqa: labels, in bd's order, made on GitHub
/// by their kind where the repo lacks them; a bd label without orqa: stays
/// in bd.
#[test]
fn a_pr_carries_its_tickets_orqa_labels() {
    let (w, o) = new_world(vec![labelled_ticket(&[
        "orqa:db",
        "bug",
        "orqa:codex-review",
    ])]);
    config(&w, &[], "");
    changed(&w, &["src/db.rs"]);
    w.hook(|_, argv| match argv {
        ["gh", "pr", "edit", .., "orqa:db"] => Some(Err("'orqa:db' not found".to_string())),
        _ => None,
    });

    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened");
    let gh: Vec<String> = w
        .called("gh ")
        .into_iter()
        .filter(|call| call.starts_with("gh label") || call.starts_with("gh pr edit"))
        .collect();
    assert_eq!(
        gh,
        [
            format!("gh pr edit {PR} --add-label orqa:db"),
            "gh label create orqa:db --color 5319E7 --description Orqadence Area label --force"
                .to_string(),
            format!("gh pr edit {PR} --add-label orqa:db"),
            format!("gh pr edit {PR} --add-label orqa:codex-review"),
        ]
    );
    assert_eq!(
        w.await_line("hx-1 PR #hx-1 not labelled"),
        format!(
            "hx-1 PR #hx-1 not labelled orqa:db: \
             gh pr edit {PR} --add-label orqa:db: exit status 1: 'orqa:db' not found"
        )
    );
    let ts = o.ticket("hx-1");
    assert!(!ts.no_review && !ts.human_merge, "state = {ts:?}");
}

/// config.json that cannot be read as the PR opens gives no kind to colour
/// the Ticket's own labels by: only the built-ins go on, the PR
/// human-merge since its labels were not read.
#[test]
fn a_config_unread_as_the_pr_opens_puts_on_no_ticket_label() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:db"])]);
    config(&w, &[], "");
    changed(&w, &["src/db.rs"]);
    let config = w.repo.join(".orqadence/config.json");
    let fix = o.run_dir("hx-1").join("fix-1.md");
    w.hook(move |_, argv| {
        if argv.starts_with(&["bd", "show"]) && fix.exists() {
            write_file(&config, "{");
        }
        None
    });

    o.run_ticket("hx-1");

    w.await_line("hx-1 Ticket labels not read, so a human merges PR #hx-1: ");
    assert_eq!(
        w.called("gh pr edit"),
        [format!("gh pr edit {PR} --add-label orqa:human-merge")]
    );
}

/// orqa:no-review on a Ticket: Implement, then a Fix that opens the PR
/// unreviewed and says why, no Review, Extra review or Debate; the PR is
/// a No-review pull request though it changes source files.
#[test]
fn a_no_review_ticket_skips_its_review_and_its_pr_gets_orqa_no_review() {
    let (w, o) = new_world(vec![labelled_ticket(&["orqa:security", "orqa:no-review"])]);
    let doc = json!({"labels": {"security": {
        "kind": "area",
        "human_merge": true,
        "extra_review": {"skill": "orqa-security-review", "position": "before_pr"},
    }}});
    write_file(&w.repo.join(".orqadence/config.json"), &doc.to_string());
    changed(&w, &["src/auth.rs"]);

    o.run_ticket("hx-1");

    w.await_line("hx-1 review 1 and debate 1 skipped: the Ticket carries orqa:no-review");
    w.await_line("hx-1 PR #hx-1 opened after 1 round");
    let prompts = w.called("herdr agent prompt");
    assert!(
        prompts
            .iter()
            .all(|p| !p.contains("review-1.md") && !p.contains("verdict-1.md")),
        "{prompts:#?}"
    );
    let fix = w.prompt("fix-1.md");
    assert!(
        fix.contains(
            "- Unreviewed: the Ticket carries orqa:no-review, the extra review skipped too"
        ),
        "{fix}"
    );
    assert_eq!(
        w.called("gh pr edit"),
        [
            format!("gh pr edit {PR} --add-label orqa:human-merge"),
            format!("gh pr edit {PR} --add-label orqa:no-review"),
            format!("gh pr edit {PR} --add-label orqa:security"),
        ]
    );
    let ts = o.ticket("hx-1");
    assert!(ts.human_merge && ts.no_review, "state = {ts:?}");
}

/// gh refuses a label the repo lacks: it is created, with its colour and
/// description, and put on. The next PR finds it there, so it is created
/// once.
#[test]
fn a_missing_github_label_is_created_once() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1"), BdTicket::new("hx-2")]);
    let created = AtomicBool::new(false);
    w.hook(move |_, argv| match argv {
        ["git", "diff", ..] => Some(Ok(MARKDOWN_AND_SKILLS.join("\0"))),
        ["gh", "label", "create", ..] => {
            created.store(true, Ordering::SeqCst);
            None
        }
        ["gh", "pr", "edit", ..] if !created.load(Ordering::SeqCst) => {
            Some(Err("'orqa:no-review' not found".to_string()))
        }
        _ => None,
    });

    o.run_ticket("hx-1");
    o.run_ticket("hx-2");

    let gh: Vec<String> = w
        .called("gh ")
        .into_iter()
        .filter(|call| call.starts_with("gh label") || call.starts_with("gh pr edit"))
        .collect();
    assert_eq!(
        gh,
        [
            "gh pr edit https://example.test/pr/hx-1 --add-label orqa:no-review",
            "gh label create orqa:no-review --color C5DEF5 --description \
             The review bots skip this pull request --force",
            "gh pr edit https://example.test/pr/hx-1 --add-label orqa:no-review",
            "gh pr edit https://example.test/pr/hx-2 --add-label orqa:no-review",
        ]
    );
    assert!(o.ticket("hx-1").no_review && o.ticket("hx-2").no_review);
}

/// gh failing to label is a line on RECENT and nothing more: the Ticket is
/// pr-open, and its PR, which the bots review, is not No-review.
#[test]
fn gh_failing_to_label_leaves_a_recent_line_and_the_ticket_pr_open() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.hook(|_, argv| match argv {
        ["git", "diff", ..] => Some(Ok(MARKDOWN_AND_SKILLS.join("\0"))),
        ["gh", "pr", "edit", ..] => Some(Err("gh: boom".to_string())),
        _ => None,
    });

    o.run_ticket("hx-1");

    assert_eq!(
        w.await_line("hx-1 PR #hx-1 not labelled"),
        format!(
            "hx-1 PR #hx-1 not labelled orqa:no-review: \
             gh pr edit {PR} --add-label orqa:no-review: exit status 1: gh: boom"
        )
    );
    w.await_line("hx-1 PR #hx-1 opened");
    let ts = o.ticket("hx-1");
    assert!(
        ts.status == STATUS_PR_OPEN && ts.pr == PR && !ts.no_review,
        "state = {ts:?}"
    );
}

/// A diff git cannot give says nothing about the PR: it is reviewed, and
/// RECENT says why.
#[test]
fn a_diff_that_cannot_be_read_is_not_no_review() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    w.fail_once("git diff --no-renames", "bad revision");

    o.run_ticket("hx-1");

    assert_eq!(
        w.await_line("hx-1 changed files not read"),
        format!("hx-1 changed files not read, so PR #hx-1 is reviewed: {DIFF}: exit status 1: bad revision")
    );
    assert_eq!(w.called("gh pr edit"), Vec::<String>::new());
    assert!(!o.ticket("hx-1").no_review);
}

/// Ticket labels bd cannot show as the PR opens may be a Human-merge
/// label's: the PR is human-merge, and RECENT says why.
#[test]
fn ticket_labels_not_read_as_the_pr_opens_make_it_human_merge() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let fix = o.run_dir("hx-1").join("fix-1.md");
    w.hook(move |_, argv| {
        (argv.starts_with(&["bd", "show"]) && fix.exists()).then(|| Err("bd down".to_string()))
    });

    o.run_ticket("hx-1");

    w.await_line("hx-1 Ticket labels not read, so a human merges PR #hx-1: ");
    assert_eq!(
        w.called("gh pr edit"),
        [format!("gh pr edit {PR} --add-label orqa:human-merge")]
    );
    let ts = o.ticket("hx-1");
    assert!(
        ts.status == STATUS_PR_OPEN && ts.human_merge && !ts.no_review,
        "state = {ts:?}"
    );
}
