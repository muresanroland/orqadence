//! Manual work: the item reader, done(), and a Stage's STATUS: manual put
//! to you as a Question, or with Away on, its Ticket parked.

use super::judgment::fake::Fake;
use super::judgment::Action;
use super::manual::{self, Item};
use super::stage::{Answer, Ask, AWAY};
use super::state::STATUS_PARKED;
use super::world::{new_world, spawn_ticket, succeed, BdTicket, Prompt};
use super::write_file;
use crate::tempdir::TempDir;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// The manual-work.md a Stage writes, its first line then the four sections.
const FILED: &str = "Ticket: hx-1 · Stage: implement · Blocks: yes

## What
Add the DEPLOY_TOKEN secret to the repo.

## Why
The deploy job reads it; the session has no credential.

## How
Run wizard.sh, then check the Actions settings page.

## Report back
The secret's name as set.
";

/// Writes a filed item to `folder`: manual-work.md and wizard.sh.
pub(crate) fn file_item(folder: &Path) {
    write_file(&folder.join("manual-work.md"), FILED);
    write_file(&folder.join("wizard.sh"), "#!/usr/bin/env bash\n");
}

#[test]
fn a_folder_reads_its_first_line_four_sections_and_forms() {
    let dir = TempDir::new();
    let folder = dir.path().join("manual-work").join("1");
    file_item(&folder);
    assert_eq!(
        manual::read(&folder).unwrap(),
        Item {
            folder: folder.clone(),
            ticket: "hx-1".to_string(),
            stage: "implement".to_string(),
            blocks: true,
            what: "Add the DEPLOY_TOKEN secret to the repo.".to_string(),
            why: "The deploy job reads it; the session has no credential.".to_string(),
            how: "Run wizard.sh, then check the Actions settings page.".to_string(),
            report_back: "The secret's name as set.".to_string(),
            wizard: true,
            prompt: false,
        }
    );
}

#[test]
fn a_missing_or_unreadable_folder_is_an_error() {
    let dir = TempDir::new();
    let folder = dir.path().join("manual-work").join("1");
    assert!(manual::read(&folder).is_err(), "a missing folder read");
    write_file(&folder.join("manual-work.md"), "## What\nsomething\n");
    assert!(manual::read(&folder).is_err(), "a first line not read");
}

#[test]
fn a_run_directory_lists_its_items_in_number_order() {
    let dir = TempDir::new();
    for n in ["10", "2", "1", "notes"] {
        file_item(&dir.path().join("manual-work").join(n));
    }
    let listed: Vec<String> = manual::list(dir.path())
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    assert_eq!(listed, ["1", "2", "10"]);
    assert!(manual::list(&dir.path().join("none")).is_empty());
}

/// Done: a bd comment on the id whose Run directory holds the item, with
/// its What and your facts, then the folder deleted.
#[test]
fn done_comments_the_what_and_facts_and_removes_the_folder() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let folder = o.run_dir("hx-1").join("manual-work").join("1");
    file_item(&folder);
    let item = manual::read(&folder).unwrap();

    manual::done(&*w, &w.repo, &item, "DEPLOY_TOKEN").unwrap();

    let comments = w.called("bd comments add hx-1 ");
    assert_eq!(
        comments,
        ["bd comments add hx-1 Manual work 1 done: Add the DEPLOY_TOKEN secret to the repo.\n\nFacts: DEPLOY_TOKEN"]
    );
    assert!(!folder.exists(), "the folder was kept");
}

/// A Stage's session that files blocking Manual work: the folder beside
/// its result file, then STATUS: manual and the folder, and waits idle.
pub(crate) fn files_manual(p: &Prompt) -> (String, String) {
    let folder = Path::new(&p.file).parent().unwrap().join("manual-work/1");
    file_item(&folder);
    (
        format!("STATUS: manual\n{}\n", folder.display()),
        "idle".to_string(),
    )
}

#[test]
fn status_manual_is_put_to_you_never_judged_and_done_sends_comments_and_deletes() {
    for (facts, sent) in [
        ("", "Manual work 1 done: "),
        ("DEPLOY_TOKEN", "Manual work 1 done: DEPLOY_TOKEN"),
    ] {
        let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
        let typesafe = Fake::down();
        o.cfg.typesafe = typesafe.clone();
        w.session(|p| match p.text.starts_with("Manual work 1 done") {
            true => (String::new(), "working".to_string()), // the answer taken up
            false if p.stage == "implement" => files_manual(p),
            false => succeed(p),
        });
        let o = Arc::new(o);
        let mut run = spawn_ticket(o.clone(), "hx-1");

        let asked = w.await_event("manual work in implement");
        assert_eq!(asked.text, "manual work in implement (pane 1-1)");
        let folder = o.run_dir("hx-1").join("manual-work").join("1");
        let Some(Ask::Manual {
            pane,
            folder: asked_folder,
            stage,
            what,
            why,
            how,
        }) = asked.ask
        else {
            panic!("STATUS: manual raised {:?}, not Manual work", asked.ask);
        };
        assert_eq!(
            (
                asked_folder,
                stage.as_str(),
                what.as_str(),
                why.as_str(),
                how.as_str()
            ),
            (
                folder.clone(),
                "implement",
                "Add the DEPLOY_TOKEN secret to the repo.",
                "The deploy job reads it; the session has no credential.",
                "Run wizard.sh, then check the Actions settings page.",
            )
        );

        o.answer("hx-1", &pane, Answer::Prompt(facts.to_string()));
        w.await_line("hx-1 sent manual work 1 done");
        write_file(&o.run_dir("hx-1").join("implement.md"), "STATUS: done\n");
        w.lock().agents.insert(pane.clone(), "idle".to_string());
        run.wait();

        w.await_line("hx-1 PR #hx-1 opened");
        assert_eq!(
            w.called(&format!("herdr agent prompt {pane} ")).last(),
            Some(&format!("herdr agent prompt {pane} {sent}")),
            "Done went into the pane as a prompt"
        );
        let comments = w.called("bd comments add hx-1 Manual work 1 done");
        assert_eq!(comments.len(), 1, "{comments:?}");
        assert_eq!(
            comments[0].contains("Facts: DEPLOY_TOKEN"),
            !facts.is_empty()
        );
        assert!(!folder.exists(), "the done item's folder was kept");
        let lines = w.lines();
        assert!(
            !lines.iter().any(|l| l.contains("stuck in")),
            "Manual work woke the Ticket: {lines:#?}"
        );
        assert!(
            typesafe.requests().is_empty(),
            "a Judgment was asked about Manual work"
        );
    }
}

#[test]
fn park_answered_on_manual_work_parks_the_ticket() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    o.cfg.typesafe = Fake::down();
    w.session(|p| match p.stage == "implement" {
        true => files_manual(p),
        false => succeed(p),
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");

    let Some(Ask::Manual { pane, .. }) = w.await_event("manual work in implement").ask else {
        panic!("no Manual work raised");
    };
    o.answer("hx-1", &pane, Answer::Act(Action::Park));
    run.wait();

    w.await_line("hx-1 parked: by you at implement");
    assert_eq!(o.ticket("hx-1").status, STATUS_PARKED);
}

#[test]
fn away_parks_manual_work_with_a_bd_comment_and_the_pane_open() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    o.cfg.away.store(true, Ordering::SeqCst);
    w.session(|p| match p.stage == "implement" {
        true => files_manual(p),
        false => succeed(p),
    });
    let o = Arc::new(o);
    spawn_ticket(o.clone(), "hx-1").wait();

    w.await_line("hx-1 parked: asked you while away");
    let ts = o.ticket("hx-1");
    assert_eq!(
        (ts.status.as_str(), ts.reason.as_str()),
        (STATUS_PARKED, AWAY)
    );
    let comments = w.called("bd comments add hx-1 ");
    assert!(
        comments.len() == 1
            && comments[0].contains("/continue @hx-1")
            && comments[0].contains("Add the DEPLOY_TOKEN secret to the repo."),
        "bd comments = {comments:?}"
    );
    assert!(
        w.called("herdr pane close").is_empty(),
        "the waiting pane was closed"
    );
    assert_eq!(w.lock().agents[&ts.panes["implement"]], "idle");
    assert!(
        w.events().iter().all(|e| e.ask.is_none()),
        "Away still put a Question"
    );
    assert!(
        o.run_dir("hx-1").join("manual-work/1").exists(),
        "the parked item's folder was deleted"
    );
}

/// A folder missing, or no folder named at all, is a Wake, once.
#[test]
fn manual_work_whose_folder_cannot_be_read_is_a_wake() {
    for filed in ["STATUS: manual\nmanual-work/9\n", "STATUS: manual\n"] {
        let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
        o.cfg.typesafe = Fake::down();
        w.session(move |p| match p.stage == "implement" {
            true => (filed.to_string(), "idle".to_string()),
            false => succeed(p),
        });
        let o = Arc::new(o);
        let mut run = spawn_ticket(o.clone(), "hx-1");

        let woke = w.await_event("stuck in implement: filed Manual work that cannot be read");
        let Some(Ask::Wake { pane, .. }) = woke.ask else {
            panic!("no Wake raised: {:?}", woke.ask);
        };
        o.answer("hx-1", &pane, Answer::Act(Action::Park));
        run.wait();
        let woke = w.lines().iter().filter(|l| l.contains("stuck in")).count();
        assert_eq!(woke, 1, "{filed:?} woke the Ticket again");
    }
}
