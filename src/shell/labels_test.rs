//! The proposed-label modal: a result's LABEL lines asked one row each,
//! before charting's outcome modal or once brainstorm-epic closes the Map;
//! accept writes the label, an existing one or none only label the Tickets
//! or leave them.

use crossterm::event::KeyCode;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use super::chart_test::{saved, started, world, writes};
use super::epic_test::last_left;
use super::shell_test::{await_line, key, render, rows, type_line};
use super::waypoint_test::live_in;
use super::Screen;
use crate::orchestrator::world::{BdTicket, World};
use crate::skills::manifest::Manifest;

const LABELLED: &str = "STATUS: done\nTICKETS: hx-1 hx-2\nLABEL: docs | Docs only | skills: o/docs-pack | tickets: hx-2\n";

fn tickets() -> Vec<BdTicket> {
    vec![BdTicket::new("hx-1"), BdTicket::new("hx-2")]
}

#[test]
fn a_label_line_opens_the_modal_before_the_tickets_modal_and_esc_keeps_it() {
    let w = world(tickets(), writes(LABELLED));
    let mut s = started(&w);

    await_line(&mut s, "hx-7 charting done: Tickets hx-1, hx-2");

    let l = s.labels.as_ref().expect("no label modal");
    let rows: Vec<_> = l
        .rows
        .iter()
        .map(|r| {
            (
                r.name.as_str(),
                r.guidance.as_str(),
                &r.skills[..],
                &r.tickets[..],
            )
        })
        .collect();
    assert_eq!(
        rows,
        [(
            "docs",
            "Docs only",
            &["o/docs-pack".to_string()][..],
            &["hx-2".to_string()][..]
        )]
    );
    assert!(s.tickets.is_none(), "the Tickets modal waits");

    s.key(key(KeyCode::Esc));

    assert!(s.labels.is_none());
    assert!(s.tickets.is_some(), "the Tickets modal opens after");
    assert_eq!(
        saved(&w).label_lines,
        ["docs | Docs only | skills: o/docs-pack | tickets: hx-2"]
    );
    s.close();
}

/// The world where charting writes LABELLED, orqa:fe and orqa:be
/// configured, and git clone finds one skill, docs-writer.
fn labelled() -> Arc<World> {
    labelled_by(LABELLED)
}

/// labelled, with charting writing `result`.
fn labelled_by(result: &'static str) -> Arc<World> {
    let w = world(tickets(), writes(result));
    let config = w.repo.join(".orqadence/config.json");
    let mut doc = config_json(&w);
    doc["labels"]["be"] = serde_json::json!({"kind": "area", "guidance": "Servers"});
    fs::write(&config, doc.to_string()).unwrap();
    w.hook(|_, argv| {
        if argv.starts_with(&["bd", "create"]) {
            return Some(Ok("hx-7\n".to_string()));
        }
        if argv.contains(&"clone") {
            let skill = Path::new(argv.last().unwrap()).join("docs-writer/SKILL.md");
            fs::create_dir_all(skill.parent().unwrap()).unwrap();
            fs::write(skill, "---\nname: docs-writer\n---\n").unwrap();
            return Some(Ok(String::new()));
        }
        argv.contains(&"rev-parse")
            .then(|| Ok("abc1234\n".to_string()))
    });
    w
}

fn config_json(w: &World) -> serde_json::Value {
    let raw = fs::read(w.repo.join(".orqadence/config.json")).unwrap();
    serde_json::from_slice(&raw).unwrap()
}

/// The label modal open on LABELLED's line.
fn asked(w: &Arc<World>) -> Screen {
    let mut s = started(w);
    await_line(&mut s, "hx-7 charting done: Tickets hx-1, hx-2");
    assert!(s.labels.is_some(), "no label modal");
    s
}

/// Tab to Apply, then Enter.
fn apply(s: &mut Screen) {
    s.key(key(KeyCode::Tab));
    s.key(key(KeyCode::Enter));
}

#[test]
fn accept_writes_the_label_installs_its_skills_and_labels_its_tickets() {
    let w = labelled();
    let mut s = asked(&w);

    apply(&mut s);

    let docs = &config_json(&w)["labels"]["docs"];
    assert_eq!(
        *docs,
        serde_json::json!({"kind": "area", "guidance": "Docs only", "skills": ["orqa-docs-writer"]})
    );
    let manifest = Manifest::load(&w.repo).unwrap();
    assert_eq!(
        manifest.skills["orqa-docs-writer"].repo,
        "https://github.com/o/docs-pack"
    );
    assert_eq!(w.called("bd label add"), ["bd label add hx-2 orqa:docs"]);
    assert!(saved(&w).label_lines.is_empty());
    assert!(s.labels.is_none());
    assert!(s.tickets.is_some(), "the Tickets modal opens after");
    await_line(
        &mut s,
        "hx-7 orqa:docs accepted: an area label in config.json, skills orqa-docs-writer; on hx-2",
    );
    s.close();
}

#[test]
fn picking_orqa_be_labels_the_tickets_with_it_and_writes_no_config() {
    let w = labelled();
    let before = config_json(&w);
    let mut s = asked(&w);

    // the picker: accept, orqa:be, orqa:fe, none
    s.key(key(KeyCode::Right));
    apply(&mut s);

    assert_eq!(w.called("bd label add"), ["bd label add hx-2 orqa:be"]);
    assert_eq!(config_json(&w), before);
    assert!(w.called("env GIT_TERMINAL_PROMPT=0 git clone").is_empty());
    assert!(saved(&w).label_lines.is_empty());
    await_line(&mut s, "hx-7 orqa:be in place of orqa:docs; on hx-2");
    s.close();
}

#[test]
fn none_writes_nothing() {
    let w = labelled();
    let before = config_json(&w);
    let mut s = asked(&w);

    s.key(key(KeyCode::Left));
    apply(&mut s);

    assert!(w.called("bd label add").is_empty());
    assert_eq!(config_json(&w), before);
    assert!(saved(&w).label_lines.is_empty(), "answered");
    await_line(&mut s, "hx-7 orqa:docs not added");
    s.close();
}

#[test]
fn a_brainstorm_epic_label_line_opens_the_modal_once_the_map_closes() {
    let result =
        "STATUS: done\nEPICS: hx-9a hx-9b\nLABEL: docs | Docs | skills: | tickets: hx-9a\n";
    let (w, mut s) = live_in(last_left(result));

    await_line(&mut s, "hx-m Map closed");
    s.tick();

    let l = s.labels.as_ref().expect("no label modal");
    assert_eq!((l.key.as_str(), l.rows.len()), ("hx-m", 1));
    s.key(key(KeyCode::Left));
    apply(&mut s);

    // its close hands over to the Epics' outcome: the Brainstorm summary
    // (harness-1n3.18), never charting's modals
    assert!(s.labels.is_none());
    assert!(s.tickets.is_none() && s.start_map.is_none());
    assert!(s.charted.is_empty());
    assert!(saved(&w).label_lines.is_empty());
    s.close();
}

#[test]
fn continue_on_a_done_brainstorm_asks_its_unanswered_lines_again() {
    let w = labelled();
    let mut s = asked(&w);
    s.key(key(KeyCode::Esc));
    s.key(key(KeyCode::Esc));
    assert!(s.tickets.is_none(), "the Tickets modal cancelled");

    type_line(&mut s, "/continue @hx-7");

    let l = s.labels.as_ref().expect("no label modal");
    assert_eq!(l.rows[0].name, "docs");
    s.key(key(KeyCode::Esc));
    assert!(s.tickets.is_none(), "no outcome after it");
    assert_eq!(saved(&w).label_lines.len(), 1);
    s.close();
}

#[test]
fn the_docs_pr_merging_after_the_answer_keeps_the_lines_answered() {
    const PR: &str = "https://github.com/o/r/pull/4";
    let w = world(
        tickets(),
        writes("STATUS: done\nTICKETS: hx-1 hx-2\nPR: https://github.com/o/r/pull/4\nLABEL: docs | Docs | skills: | tickets: hx-2\n"),
    );
    let mut s = asked(&w);
    s.key(key(KeyCode::Left));
    apply(&mut s);
    assert!(saved(&w).label_lines.is_empty());

    let merged = r#"{"state":"MERGED"}"#.to_string();
    w.lock().prs.insert(PR.to_string(), merged);
    await_line(&mut s, "hx-7 docs PR #4 merged");

    let b = saved(&w);
    assert!(b.docs_merged);
    assert!(b.label_lines.is_empty(), "{:?}", b.label_lines);
    s.close();
}

#[test]
fn the_label_modal_renders_its_row_and_the_picker_moves() {
    let w = labelled();
    let mut s = asked(&w);
    let shown = |s: &Screen| rows(&render(s, 200, 40)).join("\n");
    let text = shown(&s);
    for want in [
        " PROPOSED LABELS · 1 from hx-7 ",
        "No configured label covers these Tickets. Pick one answer for each:",
        "accept writes it to .orqadence/config.json, an existing label goes on instead, none leaves them.",
        "› orqa:docs  ‹ accept ›",
        "    Docs only",
        "    skills: o/docs-pack · for hx-2",
        "Apply",
        "Cancel",
        " ↑↓ move · ←→ picks · Tab the buttons · Enter · Esc later ",
    ] {
        assert!(text.contains(want), "{want:?} not in:\n{text}");
    }

    s.key(key(KeyCode::Right));
    s.key(key(KeyCode::Right));
    assert!(
        shown(&s).contains("› orqa:docs  ‹ orqa:fe ›"),
        "{}",
        shown(&s)
    );
    s.key(key(KeyCode::Right));
    assert!(shown(&s).contains("‹ none ›"));
    s.close();
}

#[test]
fn a_row_that_fails_keeps_its_line() {
    let w = labelled();
    w.fail_once("bd label add", "dolt is busy");
    let mut s = asked(&w);

    s.key(key(KeyCode::Right));
    apply(&mut s);

    assert_eq!(saved(&w).label_lines.len(), 1, "unanswered");
    await_line(&mut s, "hx-7 orqa:be not added: bd label add hx-2 failed");
    s.close();
}

#[test]
fn a_line_missing_its_guidance_reads_its_fields_by_their_keys() {
    let w = world(
        tickets(),
        writes("STATUS: done\nTICKETS: hx-1 hx-2\nLABEL: docs | skills: a | tickets: hx-2\n"),
    );
    let mut s = asked(&w);

    let r = &s.labels.as_ref().unwrap().rows[0];
    assert_eq!(r.guidance, "");
    assert_eq!(r.skills, ["a"]);
    assert_eq!(r.tickets, ["hx-2"]);
    s.close();
}

#[test]
fn two_rows_sharing_a_single_skill_source_both_get_its_skill() {
    let w = labelled_by(
        "STATUS: done\nTICKETS: hx-1 hx-2\n\
         LABEL: docs | Docs only | skills: o/docs-pack | tickets: hx-2\n\
         LABEL: guides | Guides | skills: o/docs-pack | tickets: hx-1\n",
    );
    let mut s = asked(&w);

    apply(&mut s);

    let labels = &config_json(&w)["labels"];
    let skills = serde_json::json!(["orqa-docs-writer"]);
    assert_eq!(labels["docs"]["skills"], skills);
    assert_eq!(labels["guides"]["skills"], skills);
    s.close();
}

#[test]
fn another_skill_installed_from_the_source_is_not_taken_for_it() {
    let w = labelled();
    w.installed("orqa-docs-other");
    let mut manifest = Manifest::load(&w.repo).unwrap();
    let other = manifest.skills.get_mut("orqa-docs-other").unwrap();
    (other.repo, other.path) = ("https://github.com/o/docs-pack".into(), "other".into());
    manifest.save(&w.repo).unwrap();
    let mut s = asked(&w);

    apply(&mut s);

    let docs = &config_json(&w)["labels"]["docs"];
    assert_eq!(docs["skills"], serde_json::json!(["orqa-docs-writer"]));
    s.close();
}

#[test]
fn the_drivers_save_during_the_answers_is_kept() {
    let w = labelled();
    let mut s = asked(&w);
    // the docs PR merges while bd labels the Tickets: the driver's save,
    // with the lines as they were on disk
    let (send, mut merged) = (s.brainstorm_sender.clone(), saved(&w));
    merged.docs_merged = true;
    let send = std::sync::Mutex::new(send);
    w.hook(move |_, argv| {
        if argv.starts_with(&["bd", "label", "add"]) {
            send.lock().unwrap().send(merged.clone()).unwrap();
        }
        None
    });

    s.key(key(KeyCode::Right));
    apply(&mut s);
    s.tick();

    let b = saved(&w);
    assert!(b.docs_merged);
    assert!(b.label_lines.is_empty(), "{:?}", b.label_lines);
    s.close();
}
