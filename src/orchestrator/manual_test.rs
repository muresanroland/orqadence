//! Manual work: the item reader, done(), and a Stage's STATUS: manual put
//! to you as a Question, or with Away on, its Ticket parked.

use super::judgment::fake::Fake;
use super::judgment::Action;
use super::manual::{self, Item};
use super::stage::{Answer, Ask, AWAY};
use super::state::STATUS_PARKED;
use super::world::{new_world, spawn_ticket, succeed, wait_until, BdTicket, Prompt};
use super::write_file;
use crate::tempdir::TempDir;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// The manual-work.md a Stage writes, its four sections.
const FILED: &str = "## What
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
fn a_folder_reads_its_sections() {
    let dir = TempDir::new();
    let folder = dir.path().join("manual-work").join("1");
    file_item(&folder);
    let item = Item {
        folder: folder.clone(),
        what: "Add the DEPLOY_TOKEN secret to the repo.".to_string(),
        why: "The deploy job reads it; the session has no credential.".to_string(),
        how: "Run wizard.sh, then check the Actions settings page.".to_string(),
        blocks: false,
    };
    assert_eq!(manual::read(dir.path(), &folder).unwrap(), item);
    let relative = Path::new("manual-work/1");
    assert_eq!(
        manual::read(dir.path(), relative).unwrap(),
        Item {
            folder: dir.path().join(relative),
            ..item
        }
    );
}

#[test]
fn a_missing_folder_is_an_error() {
    let dir = TempDir::new();
    let folder = dir.path().join("manual-work").join("1");
    assert!(
        manual::read(dir.path(), &folder).is_err(),
        "a missing folder read"
    );
    write_file(&folder.join("manual-work.md"), "## What\nsomething\n");
    assert_eq!(manual::read(dir.path(), &folder).unwrap().what, "something");
}

/// Only this Run directory's manual-work/<n>/: never another Ticket's item,
/// by ../ or an absolute path, nor a folder not named by a number.
#[test]
fn a_folder_outside_the_run_directorys_items_is_an_error() {
    let dir = TempDir::new();
    let (mine, other) = (dir.path().join("hx-1"), dir.path().join("hx-2"));
    file_item(&mine.join("manual-work/1"));
    file_item(&mine.join("manual-work/notes"));
    file_item(&other.join("manual-work/1"));
    for filed in [
        "../hx-2/manual-work/1",
        other.join("manual-work/1").to_str().unwrap(),
        "manual-work/notes",
        "manual-work",
        "",
    ] {
        assert!(
            manual::read(&mine, Path::new(filed)).is_err(),
            "{filed:?} was read"
        );
    }
}

/// manual-work itself linked to another Ticket's: its items are not this
/// Run directory's, so Done never deletes them.
#[cfg(unix)]
#[test]
fn a_linked_manual_work_folder_is_an_error() {
    let dir = TempDir::new();
    let (mine, other) = (dir.path().join("hx-1"), dir.path().join("hx-2"));
    file_item(&other.join("manual-work/1"));
    std::fs::create_dir_all(&mine).unwrap();
    std::os::unix::fs::symlink(other.join("manual-work"), mine.join("manual-work")).unwrap();
    assert!(manual::read(&mine, Path::new("manual-work/1")).is_err());
}

/// manual-work/2 linked to manual-work/1: open lists the item once.
#[cfg(unix)]
#[test]
fn open_skips_a_linked_item_folder() {
    let dir = TempDir::new();
    let items = dir.path().join("manual-work");
    file_item(&items.join("1"));
    std::os::unix::fs::symlink(items.join("1"), items.join("2")).unwrap();
    let open = manual::open(dir.path());
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0].folder, items.join("1"));
}

/// Done: a bd comment on the Ticket whose Run directory holds the item, with
/// its What and your facts, then the folder deleted.
#[test]
fn done_comments_the_what_and_facts_and_removes_the_folder() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    let folder = o.run_dir("hx-1").join("manual-work").join("1");
    file_item(&folder);
    let item = manual::read(&o.run_dir("hx-1"), &folder).unwrap();

    manual::done(&*w, &w.repo, "hx-1", &item, "DEPLOY_TOKEN").unwrap();

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
        let Some(Ask::Manual { pane, stage, item }) = asked.ask else {
            panic!("STATUS: manual raised {:?}, not Manual work", asked.ask);
        };
        assert_eq!(stage, "implement");
        assert_eq!(item, manual::read(&o.run_dir("hx-1"), &folder).unwrap());

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

/// An item filed anew while the first waits is put to you afresh, and your
/// Done goes to the one you saw: the first item stays.
#[test]
fn manual_work_filed_anew_is_put_to_you_afresh_and_done_marks_that_one() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    o.cfg.typesafe = Fake::down();
    w.session(|p| match p.text.starts_with("Manual work") {
        true => (String::new(), "working".to_string()),
        false if p.stage == "implement" => files_manual(p),
        false => succeed(p),
    });
    let o = Arc::new(o);
    let mut run = spawn_ticket(o.clone(), "hx-1");
    w.await_event("manual work in implement");

    let run_dir = o.run_dir("hx-1");
    file_item(&run_dir.join("manual-work/2"));
    write_file(
        &run_dir.join("implement.md"),
        "STATUS: manual\nmanual-work/2\n",
    );
    let Some(Ask::Manual { pane, item, .. }) = w.await_nth("manual work in implement", 2).ask
    else {
        panic!("the new item was not raised");
    };
    assert_eq!(item.folder, run_dir.join("manual-work/2"));

    o.answer("hx-1", &pane, Answer::Prompt(String::new()));
    w.await_line("hx-1 sent manual work 2 done");
    wait_until("item 2 marked done", || {
        !run_dir.join("manual-work/2").exists()
    });
    assert!(run_dir.join("manual-work/1").exists(), "item 1 was deleted");
    let comments = w.called("bd comments add hx-1 Manual work");
    assert!(
        comments.len() == 1 && comments[0].starts_with("bd comments add hx-1 Manual work 2 done"),
        "{comments:?}"
    );
    write_file(&run_dir.join("implement.md"), "STATUS: done\n");
    w.lock().agents.insert(pane.clone(), "idle".to_string());
    run.wait();
    w.await_line("hx-1 PR #hx-1 opened");
}
