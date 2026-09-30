//! The PR template the Fix that opens the PR is given: the Area label's
//! pr_template, else the default, else none, as a path in the checkout.

use super::stage::Orchestrator;
use super::world::{new_world, succeed, BdTicket, World};
use super::write_file;
use crate::setup::{DEFAULT_TEMPLATE, TEMPLATE_DIR};
use serde_json::{json, Value};
use std::sync::Arc;

/// hx-1 carrying orqa:be, an Area label with `fields` in its entry.
fn be_ticket(fields: Value) -> (Arc<World>, Orchestrator) {
    let (w, o) = new_world(vec![BdTicket {
        labels: vec!["orqa:be".to_string()],
        ..BdTicket::new("hx-1")
    }]);
    let mut be = json!({"kind": "area"});
    for (k, v) in fields.as_object().unwrap() {
        be[k] = v.clone();
    }
    let doc = json!({"labels": {"be": be}});
    write_file(&w.repo.join(".orqadence/config.json"), &doc.to_string());
    (w, o)
}

/// The Fix prompt's PR template line, or None.
fn template_input(w: &World, file: &str) -> Option<String> {
    let prompt = w.prompt(file);
    let path = prompt
        .lines()
        .find_map(|l| l.strip_prefix("- PR template: "))?;
    Some(path.to_string())
}

/// The mapped file in the checkout, given to the last Fix and not to an
/// earlier one.
#[test]
fn the_last_fix_gets_the_area_labels_pr_template_in_the_checkout() {
    let (w, o) = be_ticket(json!({"pr_template": "be.md"}));
    write_file(&w.repo.join(TEMPLATE_DIR).join("be.md"), "## What\n");
    write_file(&w.repo.join(DEFAULT_TEMPLATE), "## What\n");
    // a fix item in Round 1, so Round 2's Fix is the last
    w.session(|p| match (p.stage.as_str(), p.round) {
        ("verdict", 1) => (
            "STATUS: done\n- [fix] (medium) a.go:1 — x | reason: agreed | settled: consensus\n"
                .to_string(),
            "idle".to_string(),
        ),
        _ => succeed(p),
    });
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened after 2 rounds");
    assert_eq!(template_input(&w, "fix-1.md"), None);
    let be = w.repo.join(TEMPLATE_DIR).join("be.md");
    assert_eq!(
        template_input(&w, "fix-2.md"),
        Some(be.display().to_string())
    );
}

/// No mapping gives the default; no template at all gives no Input.
#[test]
fn no_mapping_gives_the_default_and_no_template_none() {
    let (w, o) = be_ticket(json!({}));
    write_file(&w.repo.join(DEFAULT_TEMPLATE), "## What\n");
    o.run_ticket("hx-1");
    w.await_line("hx-1 PR #hx-1 opened");
    let default = w.repo.join(DEFAULT_TEMPLATE).display().to_string();
    assert_eq!(template_input(&w, "fix-1.md"), Some(default));

    let (w, o) = be_ticket(json!({}));
    o.run_ticket("hx-1");
    w.await_line("hx-1 PR #hx-1 opened");
    assert_eq!(template_input(&w, "fix-1.md"), None);
}

/// A mapped file since removed falls back to the default, and RECENT says
/// so.
#[test]
fn a_mapped_file_removed_falls_back_to_the_default_with_a_recent_line() {
    let (w, o) = be_ticket(json!({"pr_template": "be.md"}));
    write_file(&w.repo.join(DEFAULT_TEMPLATE), "## What\n");
    o.run_ticket("hx-1");

    w.await_line(
        "hx-1 PR template .github/PULL_REQUEST_TEMPLATE/be.md is gone: the default instead",
    );
    w.await_line("hx-1 PR #hx-1 opened");
    let default = w.repo.join(DEFAULT_TEMPLATE).display().to_string();
    assert_eq!(template_input(&w, "fix-1.md"), Some(default));

    // no default either: no Input, and RECENT says there is none
    let (w, o) = be_ticket(json!({"pr_template": "be.md"}));
    o.run_ticket("hx-1");
    w.await_line(
        "hx-1 PR template .github/PULL_REQUEST_TEMPLATE/be.md is gone, and there is no default",
    );
    assert_eq!(template_input(&w, "fix-1.md"), None);
}

/// A resumed run whose PR-opening Fix is done resolves no template, so
/// RECENT says nothing of one gone.
#[test]
fn a_done_fix_resolves_no_template() {
    let (w, o) = be_ticket(json!({"pr_template": "be.md"}));
    let dir = o.run_dir("hx-1");
    for (file, text) in [
        ("implement.md", "STATUS: done\n"),
        ("review-1.md", "STATUS: done\n"),
        ("verdict-1.md", "STATUS: done\n"),
        (
            "fix-1.md",
            "STATUS: done\nPR: https://example.test/pr/hx-1\n",
        ),
    ] {
        write_file(&dir.join(file), text);
    }
    o.run_ticket("hx-1");

    w.await_line("hx-1 PR #hx-1 opened after 1 round");
    assert!(!w.lines().iter().any(|l| l.contains("PR template")));
}
