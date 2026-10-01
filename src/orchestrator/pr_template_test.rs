//! The PR template the Fix that opens the PR is given, and Address PR
//! comments too: the Area label's pr_template, else the default, else none,
//! as a path in the checkout.

use super::stage::Orchestrator;
use super::world::{new_world, spawn_epic, succeed, BdTicket, World};
use super::write_file;
use crate::setup::{DEFAULT_TEMPLATE, TEMPLATE_DIR};
use serde_json::{json, Value};
use std::sync::Arc;

/// hx-1 carrying orqa:<name>, whose label entry is `entry`.
fn area_ticket(name: &str, entry: Value) -> (Arc<World>, Orchestrator) {
    let (w, o) = new_world(vec![BdTicket {
        labels: vec![format!("orqa:{name}")],
        ..BdTicket::new("hx-1")
    }]);
    let doc = json!({"labels": {name: entry}});
    write_file(&w.repo.join(".orqadence/config.json"), &doc.to_string());
    (w, o)
}

/// The prompt's PR template line, or None.
fn template_input(w: &World, file: &str) -> Option<String> {
    w.prompt(file)
        .lines()
        .find_map(|l| l.strip_prefix("- PR template: "))
        .map(String::from)
}

/// The mapped file in the checkout, given to the last Fix and not to an
/// earlier one.
#[test]
fn the_last_fix_gets_the_area_labels_pr_template_in_the_checkout() {
    let (w, o) = area_ticket("be", json!({"kind": "area", "pr_template": "be.md"}));
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
    let (w, o) = area_ticket("be", json!({"kind": "area"}));
    write_file(&w.repo.join(DEFAULT_TEMPLATE), "## What\n");
    o.run_ticket("hx-1");
    w.await_line("hx-1 PR #hx-1 opened");
    let default = w.repo.join(DEFAULT_TEMPLATE).display().to_string();
    assert_eq!(template_input(&w, "fix-1.md"), Some(default));

    let (w, o) = area_ticket("be", json!({"kind": "area"}));
    o.run_ticket("hx-1");
    w.await_line("hx-1 PR #hx-1 opened");
    assert_eq!(template_input(&w, "fix-1.md"), None);
}

/// A mapped file since removed falls back to the default, and RECENT says
/// so.
#[test]
fn a_mapped_file_removed_falls_back_to_the_default_with_a_recent_line() {
    let (w, o) = area_ticket("be", json!({"kind": "area", "pr_template": "be.md"}));
    write_file(&w.repo.join(DEFAULT_TEMPLATE), "## What\n");
    o.run_ticket("hx-1");

    w.await_line(
        "hx-1 PR template .github/PULL_REQUEST_TEMPLATE/be.md is gone: the default instead",
    );
    w.await_line("hx-1 PR #hx-1 opened");
    let default = w.repo.join(DEFAULT_TEMPLATE).display().to_string();
    assert_eq!(template_input(&w, "fix-1.md"), Some(default));

    // no default either: no Input, and RECENT says there is none
    let (w, o) = area_ticket("be", json!({"kind": "area", "pr_template": "be.md"}));
    o.run_ticket("hx-1");
    w.await_line(
        "hx-1 PR template .github/PULL_REQUEST_TEMPLATE/be.md is gone, and there is no default",
    );
    assert_eq!(template_input(&w, "fix-1.md"), None);
}

/// Runs hx-1 to its PR, then Address PR comments on it.
fn address_pr_comments(o: Orchestrator, w: &World) {
    let o = Arc::new(o);
    let mut run = spawn_epic(o.clone(), "hx");
    w.await_line("hx-1 PR #hx-1 opened");
    o.command("address-pr-comments-hx-1");
    w.await_line("hx-1 addressed PR #hx-1");
    o.stop();
    run.wait();
    o.wait_in_flight();
}

/// Address PR comments gets the PR template as the last Fix does, and the
/// Run directory its screenshots go under: the fe label's template, and
/// with no label the default.
#[test]
fn address_pr_comments_gets_the_pr_template_as_fix_does() {
    let (w, o) = area_ticket("fe", json!({"kind": "area", "pr_template": "fe.md"}));
    write_file(&w.repo.join(TEMPLATE_DIR).join("fe.md"), "## Screenshots\n");
    write_file(&w.repo.join(DEFAULT_TEMPLATE), "## What\n");
    let run_dir = format!("- Run directory: {}", o.run_dir("hx-1").display());
    address_pr_comments(o, &w);
    let fe = w.repo.join(TEMPLATE_DIR).join("fe.md");
    assert_eq!(
        template_input(&w, "address-pr-comments.md"),
        Some(fe.display().to_string())
    );
    assert!(w.prompt("address-pr-comments.md").contains(&run_dir));

    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    write_file(&w.repo.join(DEFAULT_TEMPLATE), "## What\n");
    address_pr_comments(o, &w);
    let default = w.repo.join(DEFAULT_TEMPLATE).display().to_string();
    assert_eq!(template_input(&w, "address-pr-comments.md"), Some(default));
}
