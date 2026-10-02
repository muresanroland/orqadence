use super::herdr::{split_target, PaneInfo, PaneRect, Rect};
use super::stage::Orchestrator;
use super::state::STATUS_PR_OPEN;
use super::world::{new_world, BdTicket, World};
use super::write_file;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

fn rects(sizes: &[(usize, usize)]) -> Vec<PaneRect> {
    sizes
        .iter()
        .enumerate()
        .map(|(i, &(width, height))| PaneRect {
            pane_id: ((b'a' + i as u8) as char).to_string(),
            rect: Rect { width, height },
        })
        .collect()
}

#[test]
fn a_new_stage_pane_comes_out_of_the_roomiest_pane_along_its_longer_side() {
    // (width, height) in cells; a cell is twice as tall as it is wide, so
    // 200x100 is a square on screen and 100x100 is a tall sliver.
    let cases: [(&str, Vec<PaneRect>, &str, &str); 5] = [
        (
            "a wide pane is cut down the middle",
            rects(&[(400, 100)]),
            "a",
            "right",
        ),
        (
            "a tall pane is cut across",
            rects(&[(100, 100)]),
            "a",
            "down",
        ),
        (
            "a square pane is cut across",
            rects(&[(200, 100)]),
            "a",
            "down",
        ),
        (
            "the roomiest pane is the one cut",
            rects(&[(100, 50), (400, 100), (100, 100)]),
            "b",
            "right",
        ),
        ("no layout, no answer", Vec::new(), "", ""),
    ];
    for (name, panes, pane, direction) in cases {
        assert_eq!(
            split_target(&panes),
            (pane.to_string(), direction.to_string()),
            "{name}"
        );
    }
}

#[test]
fn a_stage_pane_is_split_the_way_the_tab_is_shaped() {
    for (name, direction, rect) in [
        ("a tall window is cut across", "down", (100, 400)),
        ("a wide window is cut down the middle", "right", (400, 50)),
    ] {
        let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
        w.lock().rect = rect;
        o.run_ticket("hx-1");

        let split = w.called("herdr pane split");
        assert!(!split.is_empty(), "{name}: no pane was split");
        assert!(
            split[0].contains(&format!("--direction {direction}")),
            "{name}: a {rect:?} pane was cut the other way: {:?}",
            split[0]
        );
    }
}

#[test]
fn a_pane_that_has_not_got_its_shell_yet_is_waited_for() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    // herdr refuses until the pane it just created has a shell.
    w.fail_once(
        "herdr agent start",
        r#"{"error":{"code":"agent_pane_busy","message":"agent target w1:p2 is not an available shell"}}"#,
    );

    o.run_ticket("hx-1");

    let got = o.ticket("hx-1");
    assert_eq!(
        got.status, STATUS_PR_OPEN,
        "a pane that was a moment from ready ended the Stage: {got:?}"
    );
}

#[test]
fn a_session_still_picking_up_its_prompt_is_not_a_finished_stage() {
    let (w, mut o) = new_world(vec![BdTicket::new("hx-1")]);
    o.cfg.tick = Duration::from_millis(10); // a 30ms grace for the session to get going
    w.session(|p| {
        // Reads idle at once, as a real session does, and writes its result
        // a moment later.
        let file = PathBuf::from(&p.file);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(5));
            write_file(&file, "STATUS: done\nPR: https://example.test/pr\n");
        });
        (String::new(), "idle".to_string())
    });

    o.run_ticket("hx-1");

    for line in w.lines() {
        assert!(
            !line.contains("stuck in"),
            "a session that was still starting was woken on: {line:?}"
        );
    }
    let got = o.ticket("hx-1");
    assert_eq!(
        got.status, STATUS_PR_OPEN,
        "the Pipeline did not finish: {got:?}"
    );
}

#[test]
fn a_tickets_stage_panes_go_in_its_own_tab_split_by_the_layout() {
    let (w, o) = new_world(vec![BdTicket::new("hx-1")]);
    o.run_ticket("hx-1");

    let repo = w.repo.display().to_string();
    let got: Vec<String> = w
        .calls()
        .into_iter()
        .filter(|c| {
            [
                "herdr tab create",
                "herdr pane list",
                "herdr pane layout",
                "herdr pane split",
            ]
            .iter()
            .any(|p| c.starts_with(p))
        })
        .map(|c| c.replace(&repo, "<repo>"))
        .collect();
    let worktree = "--cwd <repo>/.orqadence-local/worktrees/hx-1 --no-focus";
    let run = "--cwd <repo>/.orqadence-local/runs/hx-1 --no-focus";
    // pane list: the placement's read of the tab, and the event line's
    // locate after each new pane.
    let list = "herdr pane list --workspace w1".to_string();
    assert_eq!(
        got,
        [
            format!("herdr tab create --workspace w1 --label hx-1 {worktree}"),
            list.clone(),
            list.clone(),
            "herdr pane layout --pane w1:p2".to_string(),
            format!("herdr pane split w1:p2 --direction down {run}"),
            list.clone(),
            list.clone(),
            "herdr pane layout --pane w1:p2".to_string(),
            format!(
                "herdr pane split w1:p2 --direction down {worktree} --env TYPESAFE_API_KEY=sk-test"
            ),
            list.clone(),
            list.clone(),
            "herdr pane layout --pane w1:p2".to_string(),
            format!("herdr pane split w1:p2 --direction down {worktree}"),
            list,
        ]
    );
}

#[test]
fn a_research_tab_is_created_once_and_split_on_the_next_call() {
    let (w, o) = new_world(Vec::new());
    let placement = ["--cwd", "/r", "--no-focus"];

    let (tab, first) = o.place_pane("", "research-m1", &placement, "").unwrap();
    let (again, second) = o.place_pane(&tab, "research-m1", &placement, "").unwrap();

    assert_eq!(
        w.called("herdr tab create"),
        ["herdr tab create --workspace w1 --label research-m1 --cwd /r --no-focus"]
    );
    assert_eq!(again, tab, "the second pane went to another tab");
    assert_eq!(
        w.called("herdr pane split"),
        [format!(
            "herdr pane split {first} --direction down --cwd /r --no-focus"
        )]
    );
    assert_ne!(second, first);
}

#[test]
fn the_excluded_pane_is_never_split_even_when_the_layout_cannot_be_read() {
    let (w, o) = new_world(Vec::new());
    let (tab, a) = o.place_pane("", "t", &[], "").unwrap();
    let (_, b) = o.place_pane(&tab, "t", &[], "").unwrap();

    for (name, never, layout_fails) in [
        (
            "a failed layout read falls back to the last pane",
            b.as_str(),
            true,
        ),
        ("split_target's pick", a.as_str(), false),
    ] {
        let before = w.calls().len();
        if layout_fails {
            w.fail_once("herdr pane layout", "boom");
        }
        o.place_pane(&tab, "t", &[], never).unwrap();
        let split = w.since(before, "herdr pane split");
        assert_eq!(split.len(), 1, "{name}: {split:?}");
        assert!(
            !split[0].starts_with(&format!("herdr pane split {never} ")),
            "{name}: the excluded pane was split: {split:?}"
        );
    }
}

/// A world whose Shell runs in pane w1:shell of tab w1:t0, beside a pane the
/// user opened there.
fn shell_world() -> (Arc<World>, Orchestrator) {
    let (w, o) = new_world(Vec::new());
    {
        let mut inner = w.lock();
        inner.tabs.push("w1:t0".to_string());
        for id in ["w1:shell", "w1:mine"] {
            inner.panes.push(PaneInfo {
                pane_id: id.to_string(),
                tab_id: "w1:t0".to_string(),
            });
        }
    }
    (w, o)
}

#[test]
fn the_first_brainstorm_pane_splits_the_shells_pane_once_with_a_ratio() {
    let (w, o) = shell_world();

    let pane = o
        .place_beside_shell("w1:shell", "", &["--no-focus"])
        .unwrap();

    assert_eq!(
        w.called("herdr pane get w1:shell").len(),
        1,
        "the tab was not read live"
    );
    let split = w.called("herdr pane split");
    assert_eq!(split.len(), 1, "{split:?}");
    assert!(
        split[0].starts_with("herdr pane split w1:shell ") && split[0].contains(" --ratio "),
        "{split:?}"
    );
    let got = w.lock().panes.iter().find(|p| p.pane_id == pane).cloned();
    assert_eq!(got.map(|p| p.tab_id).as_deref(), Some("w1:t0"));
}

#[test]
fn a_later_brainstorm_pane_replaces_the_previous_one() {
    let (w, o) = shell_world();
    let first = o
        .place_beside_shell("w1:shell", "", &["--no-focus"])
        .unwrap();
    let before = w.calls().len();

    let second = o
        .place_beside_shell("w1:shell", &first, &["--no-focus"])
        .unwrap();

    assert_eq!(
        w.since(before, "herdr pane split"),
        [format!(
            "herdr pane split {first} --direction right --no-focus"
        )],
        "only the previous Brainstorm pane may be split"
    );
    assert_eq!(
        w.since(before, "herdr pane close"),
        [format!("herdr pane close {first}")]
    );
    let panes: Vec<String> = w.lock().panes.iter().map(|p| p.pane_id.clone()).collect();
    assert_eq!(panes, ["w1:shell", "w1:mine", second.as_str()]);
}
