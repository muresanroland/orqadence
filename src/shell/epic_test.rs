//! The Brainstorm's end over the fake world: brainstorm-epic once every
//! other Waypoint of the Map is closed, its Epics closing the Map, and the
//! docs PR tracked until it merges.

use std::sync::Arc;

use super::chart_test::{await_prompt, map, pane_alive, saved, world};
use super::shell_test::{await_line, type_line};
use super::waypoint_test::live_in as live;
use crate::brainstorm::Phase;
use crate::orchestrator::app::{self, RELEASE_ON};
use crate::orchestrator::state::LOCAL;
use crate::orchestrator::world::{BdTicket, Prompt, World};

const PR: &str = "https://github.com/o/r/pull/71";

/// Charting writes Map hx-m; brainstorm-epic writes `epic` and goes idle.
fn ends(epic: &'static str) -> impl Fn(&Prompt) -> (String, String) + Send + Sync {
    move |p: &Prompt| match p.stage.as_str() {
        "chart" => ("STATUS: done\nMAP: hx-m\n".to_string(), "idle".to_string()),
        "epic" => (epic.to_string(), "idle".to_string()),
        _ => (String::new(), "idle".to_string()),
    }
}

/// Map hx-m with every Waypoint closed but its build-Epic one, hx-m.e1,
/// beside Epics hx-9a and hx-9b and a closed hx-9c; brainstorm-epic writes
/// `epic`.
fn last_left(epic: &'static str) -> Arc<World> {
    let mut issues = map(1);
    issues.extend(["hx-9a", "hx-9b", "hx-9c"].map(BdTicket::new));
    let w = world(issues, ends(epic));
    for t in w.lock().tickets.iter_mut() {
        if t.id.starts_with("hx-9") {
            (t.issue_type, t.no_epic) = ("epic".to_string(), true);
        }
        if (t.id.starts_with("hx-m.") && t.id != "hx-m.e1") || t.id == "hx-9c" {
            t.status = "closed".to_string();
        }
    }
    w
}

/// The prompt the brainstorm-epic session took.
fn epic_prompt(w: &World) -> String {
    await_prompt(w, "# Brainstorm Epics")
}

#[test]
fn the_build_epic_waypoint_never_starts_while_another_is_open() {
    // hx-m.1 claimed elsewhere: the frontier holds only the unblocked
    // build-Epic Waypoint
    let w = last_left("");
    w.lock()
        .tickets
        .iter_mut()
        .find(|t| t.id == "hx-m.1")
        .unwrap()
        .status = "in_progress".to_string();
    let (w, mut s) = live(w);

    await_line(&mut s, "hx-m waiting: no Waypoint is ready for you");

    let starts = w.called("herdr agent start");
    assert!(starts.iter().all(|c| !c.contains("-epic")), "{starts:?}");
    s.close();
}

#[test]
fn every_other_waypoint_closed_starts_brainstorm_epic_with_its_inputs() {
    let w = last_left("");
    app::set_switch(&w.repo, &RELEASE_ON, true).unwrap();
    let (w, mut s) = live(w);

    await_line(&mut s, "hx-m.e1 Waypoint started");

    let result = w.repo.join(LOCAL).join("brainstorms/hx-7/epic.md");
    let prompt = epic_prompt(&w);
    for want in [
        "- MAP: hx-m\n",
        "- WAYPOINT: hx-m.e1\n",
        "- TICKET LABELS: orqa:fe (area): Screens and styles\n",
        "- RELEASES: on\n",
        &format!("- RESULT FILE: {}\n", result.display()),
    ] {
        assert!(prompt.contains(want), "{want:?} not in:\n{prompt}");
    }
    let start = w.called("herdr agent start").pop().unwrap();
    assert!(start.contains("h-hx-7-epic"), "{start}");
    s.close();
}

#[test]
fn releases_is_off_with_the_release_switch_off() {
    let (w, mut s) = live(last_left(""));

    await_line(&mut s, "hx-m.e1 Waypoint started");

    assert!(epic_prompt(&w).contains("- RELEASES: off\n"));
    s.close();
}

#[test]
fn a_result_naming_open_epics_closes_the_map_and_removes_the_worktree() {
    let result = "STATUS: done\nEPICS: hx-9a hx-9b\nPR: https://github.com/o/r/pull/71\n";
    let (w, mut s) = live(last_left(result));

    await_line(&mut s, "hx-m.e1 closed: wrote Epics hx-9a, hx-9b");
    await_line(&mut s, &format!("hx-m docs PR #71 opened: {PR}"));
    await_line(
        &mut s,
        "hx-m Map closed, worktree removed; /start-epic hx-9a once PR #71 merges",
    );

    let close = w.called("bd close hx-m ").pop().unwrap();
    assert!(
        close.contains("hx-9a") && close.contains("hx-9b"),
        "{close}"
    );
    assert_eq!(w.called("git worktree remove").len(), 1);
    assert!(w.called("git branch -D").is_empty(), "its branch is kept");
    let b = saved(&w);
    assert_eq!(b.phase, Phase::Done);
    assert_eq!(b.epics, ["hx-9a", "hx-9b"]);
    assert_eq!(b.docs_pr, PR);
    assert_eq!(s.live, None);
    s.close();
}

#[test]
fn a_result_naming_a_closed_epic_keeps_the_pane() {
    let (w, mut s) = live(last_left("STATUS: done\nEPICS: hx-9a hx-9c\n"));

    await_line(
        &mut s,
        "hx-m Epics result not taken: hx-9c is not an open epic",
    );

    let b = saved(&w);
    assert!(pane_alive(&w, &b.pane), "its pane is kept");
    assert_eq!(b.phase, Phase::Map);
    assert!(w.called("bd close hx-m ").is_empty());
    s.close();
}

#[test]
fn the_docs_pr_merged_suggests_start_epic_of_the_first_epic() {
    let result = "STATUS: done\nEPICS: hx-9a hx-9b\nPR: https://github.com/o/r/pull/71\n";
    let (w, mut s) = live(last_left(result));
    await_line(&mut s, "hx-m Map closed");
    assert_eq!(s.suggestion, None);

    let merged = r#"{"state":"MERGED"}"#.to_string();
    w.lock().prs.insert(PR.to_string(), merged);

    await_line(&mut s, "hx-m docs PR #71 merged: /start-epic hx-9a is next");
    assert_eq!(s.suggestion.as_deref(), Some("/start-epic hx-9a"));
    assert!(saved(&w).docs_merged);
    s.close();
}

#[test]
fn the_docs_pr_closed_unmerged_is_kept_and_its_merge_after_a_reopen_seen() {
    let result = "STATUS: done\nEPICS: hx-9a hx-9b\nPR: https://github.com/o/r/pull/71\n";
    let (w, mut s) = live(last_left(result));
    await_line(&mut s, "hx-m Map closed");

    let closed = r#"{"state":"CLOSED"}"#.to_string();
    w.lock().prs.insert(PR.to_string(), closed);

    await_line(&mut s, "hx-m docs PR #71 closed unmerged");
    assert_eq!(s.suggestion, None);
    assert_eq!(saved(&w).docs_pr, PR, "kept until it merges");

    let merged = r#"{"state":"MERGED"}"#.to_string();
    w.lock().prs.insert(PR.to_string(), merged);

    await_line(&mut s, "hx-m docs PR #71 merged: /start-epic hx-9a is next");
    assert_eq!(s.suggestion.as_deref(), Some("/start-epic hx-9a"));
    s.close();
}

#[test]
fn the_map_not_closing_keeps_the_brainstorm_on_its_map_and_tries_again() {
    let w = last_left("STATUS: done\nEPICS: hx-9a hx-9b\n");
    w.fail_once("bd close hx-m ", "dolt is busy");
    let (w, mut s) = live(w);

    await_line(&mut s, "hx-m Epics result not taken: Map not closed");
    await_line(&mut s, "hx-m Map closed, worktree removed");

    assert_eq!(w.called("bd close hx-m ").len(), 2);
    assert_eq!(saved(&w).phase, Phase::Done);
    s.close();
}

#[test]
fn start_epic_before_the_docs_pr_merges_warns() {
    let result = "STATUS: done\nEPICS: hx-9a hx-9b\nPR: https://github.com/o/r/pull/71\n";
    let (_w, mut s) = live(last_left(result));
    await_line(&mut s, "hx-m Map closed");

    type_line(&mut s, "/start-epic hx-9b");

    assert!(s.run.is_some(), "the Epic run starts all the same");
    let want = format!("docs PR {PR} not merged; Tickets won't see its docs");
    assert!(
        s.notices.iter().any(|n| n.text == want),
        "no notice {want:?}"
    );
    s.close();
}

#[test]
fn no_docs_pr_suggests_start_epic_at_once() {
    let (_w, mut s) = live(last_left("STATUS: done\nEPICS: hx-9a hx-9b\n"));

    await_line(
        &mut s,
        "hx-m Map closed, worktree removed; /start-epic hx-9a is next",
    );

    assert_eq!(s.suggestion.as_deref(), Some("/start-epic hx-9a"));
    s.close();
}
