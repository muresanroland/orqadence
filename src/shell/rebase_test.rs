//! The Brainstorm's branch rebased on /continue over the fake world, git
//! answered by a hook: rebased before the session starts, a conflict
//! aborted and asked about, skipped while research runs.

use crossterm::event::KeyCode;
use std::sync::{Arc, Mutex};

use super::chart_test::saved;
use super::shell_test::{await_line, key};
use super::waypoint_test::{closed, stopped, working};
use super::Screen;
use crate::brainstorm::research::{self, Update};
use crate::brainstorm::Research;
use crate::orchestrator::world::World;

/// git in the Brainstorm's worktree: `behind` commits on main the branch
/// lacks, `rebase` what git rebase gives, `files` the ones it left
/// conflicted; none left, git refused before starting and has no rebase
/// to abort.
fn git(
    w: &World,
    behind: &'static str,
    rebase: Result<&'static str, &'static str>,
    files: &'static str,
) {
    let worktree = saved(w).worktree;
    w.hook(move |dir, argv| {
        let cmd = argv.join(" ");
        if !cmd.starts_with("git ") {
            return None;
        }
        assert_eq!(dir.display().to_string(), worktree, "{cmd}");
        match cmd.as_str() {
            "git rev-list --count HEAD..origin/main" => Some(Ok(format!("{behind}\n"))),
            "git rebase origin/main" => Some(rebase.map(str::to_string).map_err(str::to_string)),
            "git diff --name-only --diff-filter=U" => Some(Ok(files.to_string())),
            "git rebase --abort" if files.is_empty() => {
                Some(Err("fatal: No rebase in progress?".to_string()))
            }
            _ => None,
        }
    });
}

/// The index of the first call since call `before` that starts with `prefix`.
fn call_index(w: &World, before: usize, prefix: &str) -> usize {
    let calls = w.calls()[before..].to_vec();
    calls
        .iter()
        .position(|c| c.starts_with(prefix))
        .unwrap_or_else(|| panic!("never called {prefix:?}: {calls:?}"))
}

#[test]
fn a_diverged_branch_is_rebased_before_the_session_starts() {
    let (w, mut s) = stopped();
    git(&w, "3", Ok(""), "");
    let before = w.calls().len();

    s.command("/continue @hx-m.3");

    await_line(&mut s, "hx-m brainstorm/hx-7 rebased on main (3 commits)");
    await_line(&mut s, "hx-m.3 Waypoint started");
    let fetch = call_index(&w, before, "git fetch origin");
    let rebase = call_index(&w, before, "git rebase origin/main");
    let start = call_index(&w, before, "herdr agent start");
    assert!(fetch < rebase && rebase < start, "{:?}", w.calls());
    s.close();
}

/// /continue @hx-m.3 over a rebase that conflicts in CONTEXT.md: the
/// Question waits.
fn conflicted() -> (Arc<World>, Screen) {
    let (w, mut s) = stopped();
    git(
        &w,
        "3",
        Err("CONFLICT (content): Merge conflict in CONTEXT.md"),
        "CONTEXT.md\n",
    );
    s.command("/continue @hx-m.3");
    await_line(
        &mut s,
        "hx-m rebasing brainstorm/hx-7 on main conflicts in CONTEXT.md: aborted, asking you",
    );
    (w, s)
}

#[test]
fn a_rebase_that_conflicts_is_aborted_and_asks_naming_the_files() {
    let (w, mut s) = conflicted();

    assert_eq!(w.called("git rebase --abort").len(), 1);
    let q = &s.questions[0];
    assert_eq!(
        q.text,
        "hx-m: rebasing brainstorm/hx-7 on main conflicts (CONTEXT.md)"
    );
    assert!(q.brainstorms(), "no run's end drops it");
    assert_eq!(
        s.options(),
        [
            "resolve it yourself in .orqadence-local/worktrees/hx-7, then /continue @hx-m.3",
            "carry on from the old base",
        ]
    );
    assert!(!waypoint_started(&w), "no session before the answer");
    s.close();
}

#[test]
fn resolve_it_yourself_starts_no_session() {
    let (w, mut s) = conflicted();

    s.key(key(KeyCode::Enter));

    assert!(s.questions.is_empty());
    assert_eq!(s.live, None, "the Brainstorm stays stopped");
    assert_eq!(s.suggestion.as_deref(), Some("/continue @hx-m.3"));
    assert!(!waypoint_started(&w));
    s.close();
}

#[test]
fn carry_on_starts_the_session_on_the_old_base() {
    let (w, mut s) = conflicted();
    let rebases = w.called("git rebase origin/main").len();

    s.key(key(KeyCode::Down));
    s.key(key(KeyCode::Enter));

    await_line(&mut s, "hx-m.3 Waypoint started");
    assert_eq!(w.called("git rebase origin/main").len(), rebases);
    assert!(s.questions.is_empty());
    s.close();
}

#[test]
fn a_running_research_session_skips_the_rebase_with_its_line() {
    let (w, mut s) = stopped();
    git(&w, "3", Ok(""), "");
    let mut b = saved(&w);
    b.research = vec![Research {
        waypoint: "hx-m.4".to_string(),
        pane: "w9:p9".to_string(),
        ..Default::default()
    }];
    b.save(&w.repo).unwrap();
    if let Some(saved) = s.brainstorms.iter_mut().find(|s| s.idea == b.idea) {
        *saved = b;
    }

    s.command("/continue @hx-m");

    await_line(
        &mut s,
        "hx-m rebase skipped: research 4 still running, the next /continue tries again",
    );
    assert!(w.called("git fetch").is_empty());
    assert!(w.called("git rebase").is_empty());
    let m = s.start_map.as_ref().expect("the Continue form");
    assert_eq!(m.rebase, "rebase skipped: research 4 still running");
    s.close();
}

#[test]
fn a_branch_not_diverged_runs_no_rebase() {
    let (w, mut s) = stopped();
    git(&w, "0", Ok(""), "");

    s.command("/continue @hx-m.3");

    await_line(&mut s, "hx-m.3 Waypoint started");
    assert_eq!(w.called("git fetch origin").len(), 1);
    assert!(w.called("git rebase").is_empty());
    s.close();
}

/// Whether a Waypoint session started since the Map stopped: one beyond
/// hx-m.1's, which stopped() ran.
fn waypoint_started(w: &World) -> bool {
    let starts = w.called("herdr agent start");
    starts.iter().filter(|c| c.contains("-waypoint")).count() > 1
}

#[test]
fn a_rebase_that_fails_without_conflicts_is_aborted_and_the_session_starts() {
    let (w, mut s) = stopped();
    git(
        &w,
        "3",
        Err("cannot rebase: You have unstaged changes."),
        "",
    );

    s.command("/continue @hx-m.3");

    await_line(&mut s, "hx-m rebase skipped: git rebase failed:");
    await_line(&mut s, "hx-m.3 Waypoint started");
    assert_eq!(w.called("git rebase --abort").len(), 1);
    assert!(s.questions.is_empty());
    s.close();
}

/// A parked Research Waypoint of hx-m's, its session saved.
fn parked(id: &str) -> Research {
    let mut r = Research {
        waypoint: id.to_string(),
        parked: true,
        ..Default::default()
    };
    r.session.id = format!("sess-{id}");
    r
}

#[test]
fn research_sent_during_the_rebase_is_kept_by_the_waypoint_started() {
    let (w, mut s) = stopped();
    git(&w, "3", Ok(""), "");
    let update = Update {
        idea: "hx-7".to_string(),
        tab: "w1:t9".to_string(),
        research: vec![parked("hx-m.4")],
        closed: None,
    };
    s.research_sender.send(update).unwrap();

    s.command("/continue @hx-m.3");

    await_line(&mut s, "hx-m.3 Waypoint started");
    let b = saved(&w);
    assert_eq!(b.research, [parked("hx-m.4")]);
    assert_eq!(b.research_tab, "w1:t9");
    let live = s.brainstorms.iter().find(|b| b.idea == "hx-7").unwrap();
    assert_eq!(live.research, [parked("hx-m.4")]);
    s.close();
}

#[test]
fn continue_at_a_parked_research_waypoint_rebases_before_it_resumes() {
    let (w, mut s) = stopped();
    git(&w, "3", Ok(""), "");
    let mut b = saved(&w);
    b.research = vec![parked("hx-m.3")];
    b.save(&w.repo).unwrap();
    if let Some(saved) = s.brainstorms.iter_mut().find(|s| s.idea == b.idea) {
        *saved = b;
    }

    s.command("/continue @hx-m.3");

    await_line(&mut s, "hx-m brainstorm/hx-7 rebased on main (3 commits)");
    await_line(&mut s, "hx-m.3 research resumes");
    s.close();
}

#[test]
fn the_fetch_and_the_rebase_hold_the_research_gate() {
    let (w, mut s) = stopped();
    // the research thread the live Map started, or one in its place once
    // a poll dropped it ended
    if !s.research.iter().any(|r| r.idea == "hx-7") {
        let (stop, sender) = (s.brainstorm_stop.clone(), s.research_sender.clone());
        let h = research::spawn(&s.cfg, saved(&w), false, stop, sender);
        s.research.push(h);
    }
    let gate = s
        .research
        .iter()
        .find(|r| r.idea == "hx-7")
        .unwrap()
        .gate
        .clone();
    let held = Arc::new(Mutex::new(Vec::new()));
    let seen = held.clone();
    w.hook(move |_, argv| match argv.join(" ").as_str() {
        "git fetch origin" | "git rebase origin/main" => {
            seen.lock().unwrap().push(gate.try_lock().is_err());
            Some(Ok(String::new()))
        }
        "git rev-list --count HEAD..origin/main" => Some(Ok("3\n".to_string())),
        _ => None,
    });

    s.command("/continue @hx-m.3");

    await_line(&mut s, "hx-m.3 Waypoint started");
    assert_eq!(*held.lock().unwrap(), [true, true]);
    s.close();
}

#[test]
fn continue_at_parked_research_while_its_map_session_runs_skips_the_rebase() {
    let (w, mut s, _) = working();
    git(&w, "3", Ok(""), "");
    let mut b = saved(&w);
    b.research = vec![parked("hx-m.3")];
    b.save(&w.repo).unwrap();
    if let Some(saved) = s.brainstorms.iter_mut().find(|s| s.idea == b.idea) {
        *saved = b;
    }

    s.command("/continue @hx-m.3");

    await_line(
        &mut s,
        "hx-m rebase skipped: a Waypoint session of hx-m is running",
    );
    await_line(&mut s, "hx-m.3 research resumes");
    assert!(w.called("git fetch").is_empty());
    assert!(w.called("git rebase").is_empty());
    s.close();
}

#[test]
fn resolve_it_yourself_on_the_live_map_stops_it() {
    let (w, mut s, _) = closed();
    git(
        &w,
        "3",
        Err("CONFLICT (content): Merge conflict in CONTEXT.md"),
        "CONTEXT.md\n",
    );
    s.command("/continue @hx-m.3");
    await_line(
        &mut s,
        "hx-m rebasing brainstorm/hx-7 on main conflicts in CONTEXT.md: aborted, asking you",
    );
    let starts = w.called("herdr agent start").len();
    // the rebase Question answered ahead of the pending Next Waypoint?
    let q = s.questions.pop().unwrap();
    s.questions.insert(0, q);

    s.key(key(KeyCode::Enter));
    assert_eq!(s.live, None);
    s.key(key(KeyCode::Enter));

    await_line(&mut s, "hx-m not acted on: the Map is no longer live");
    assert_eq!(w.called("herdr agent start").len(), starts);
    s.close();
}
