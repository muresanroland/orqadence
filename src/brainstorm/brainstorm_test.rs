use super::{load, most_recent, Brainstorm, Phase, Research};
use crate::orchestrator::scheduler::BdIssue;
use crate::orchestrator::state::Session;
use crate::tempdir::TempDir;

fn session(id: &str) -> Session {
    Session {
        app: "claude".to_string(),
        id: id.to_string(),
        reset: None,
    }
}

/// Every field filled, so a field that does not round-trip shows.
fn mapped(idea: &str, map: &str) -> Brainstorm {
    Brainstorm {
        phase: Phase::Map,
        idea: idea.to_string(),
        map: map.to_string(),
        pane: "1-2".to_string(),
        session: Some(session("s-1")),
        worktree: format!("/repo/.orqadence-local/worktrees/{idea}"),
        branch: format!("brainstorm/{idea}"),
        background: true,
        research_tab: format!("research-{map}"),
        research: vec![Research {
            waypoint: format!("{map}.3"),
            pane: "5-1".to_string(),
            session: session("s-2"),
            parked: true,
            limited: true,
        }],
        label_lines: vec!["LABEL: docs | the docs | skills: a | tickets: hx-1".to_string()],
        epics: vec!["hx-e1".to_string()],
        docs_pr: "https://github.com/o/r/pull/71".to_string(),
        docs_merged: true,
    }
}

fn epic(id: &str, status: &str) -> BdIssue {
    BdIssue {
        id: id.to_string(),
        status: status.to_string(),
        issue_type: "epic".to_string(),
        labels: vec![super::MAP.to_string()],
        ..Default::default()
    }
}

/// state.json round-trips, written through a temp file renamed over it.
#[test]
fn a_saved_brainstorm_loads_as_it_was_and_no_temp_file_is_left() {
    let repo = TempDir::new();
    let b = mapped("hx-1", "hx-2");

    b.save(repo.path()).unwrap();
    b.save(repo.path()).unwrap();

    assert_eq!(load(repo.path(), &[]), [b]);
    let dir = repo.path().join(".orqadence-local/brainstorms/hx-1");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    assert_eq!(files, ["state.json"]);
}

/// Two folders are two saved Brainstorms, in idea order.
#[test]
fn two_folders_load_as_two_saved_brainstorms() {
    let repo = TempDir::new();
    let charting = Brainstorm {
        idea: "hx-7".to_string(),
        ..Default::default()
    };
    charting.save(repo.path()).unwrap();
    mapped("hx-1", "hx-2").save(repo.path()).unwrap();

    let ideas: Vec<String> = load(repo.path(), &[]).into_iter().map(|b| b.idea).collect();

    assert_eq!(ideas, ["hx-1", "hx-7"]);
}

/// A Map the user closed in bd throws its Brainstorm away; one whose Map is
/// open, or a done one whose Map Orqadence closed, stays.
#[test]
fn one_whose_map_is_closed_in_bd_is_not_listed() {
    let repo = TempDir::new();
    mapped("hx-1", "hx-2").save(repo.path()).unwrap();
    mapped("hx-3", "hx-4").save(repo.path()).unwrap();
    Brainstorm {
        phase: Phase::Done,
        ..mapped("hx-5", "hx-6")
    }
    .save(repo.path())
    .unwrap();
    let issues = [
        epic("hx-2", "closed"),
        epic("hx-4", "open"),
        epic("hx-6", "closed"),
    ];

    let ideas: Vec<String> = load(repo.path(), &issues)
        .into_iter()
        .map(|b| b.idea)
        .collect();

    assert_eq!(ideas, ["hx-3", "hx-5"]);
}

/// While charting, the Idea closed in bd throws it away.
#[test]
fn a_charting_one_whose_idea_is_closed_in_bd_is_not_listed() {
    let repo = TempDir::new();
    Brainstorm {
        idea: "hx-1".to_string(),
        ..Default::default()
    }
    .save(repo.path())
    .unwrap();
    let idea = BdIssue {
        id: "hx-1".to_string(),
        status: "closed".to_string(),
        labels: vec![super::IDEA.to_string()],
        ..Default::default()
    };

    assert_eq!(load(repo.path(), &[idea]), []);
}

/// Sets the Brainstorm's state.json mtime to `secs` after the epoch.
fn touch(repo: &std::path::Path, idea: &str, secs: u64) {
    let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs);
    let file = repo
        .join(".orqadence-local/brainstorms")
        .join(idea)
        .join("state.json");
    let file = std::fs::File::options().write(true).open(file).unwrap();
    file.set_modified(at).unwrap();
}

/// The most recent is the one whose state.json changed last; a done one is
/// never it, and with none saved there is none.
#[test]
fn the_most_recent_is_the_last_saved_but_never_a_done_one() {
    let repo = TempDir::new();
    let charting = Brainstorm {
        idea: "hx-7".to_string(),
        ..Default::default()
    };
    charting.save(repo.path()).unwrap();
    mapped("hx-1", "hx-2").save(repo.path()).unwrap();
    Brainstorm {
        phase: Phase::Done,
        ..mapped("hx-5", "hx-6")
    }
    .save(repo.path())
    .unwrap();
    touch(repo.path(), "hx-7", 2_000);
    touch(repo.path(), "hx-1", 1_000);
    touch(repo.path(), "hx-5", 3_000);
    let saved = load(repo.path(), &[]);

    let recent = most_recent(repo.path(), &saved).map(Brainstorm::key);
    assert_eq!(recent, Some("hx-7"));

    touch(repo.path(), "hx-1", 2_500);
    let recent = most_recent(repo.path(), &saved).map(Brainstorm::key);
    assert_eq!(recent, Some("hx-2"), "a Map's key is the Map");

    assert_eq!(most_recent(repo.path(), &[]), None);
}
