//! /brainstorm's idea modal: typing, Tab, Esc and Cancel, Ctrl+G through
//! the fake editor, Start over fake Tools, and its render beside the Shell
//! and folded over it.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::fs;
use std::path::Path;
use std::sync::Arc;

use super::brand::GREEN;
use super::shell_test::{find, key, line, notice, render, row, screen_at, type_in, type_line};
use super::Screen;
use crate::brainstorm::{Brainstorm, Phase};
use crate::orchestrator::state::LOCAL;
use crate::tempdir::TempDir;
use crate::tools::fake::{Fake, FakeEditor};

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// The Shell with the idea modal open, over `tools` in `repo`.
fn opened(tools: Arc<Fake>, repo: &Path) -> Screen {
    let mut s = screen_at(tools, repo);
    type_line(&mut s, "/brainstorm");
    s
}

fn text(s: &Screen) -> &str {
    &s.idea.as_ref().expect("the idea modal is closed").text
}

/// The env an editor is picked from: VISUAL=vi.
fn visual(k: &str) -> String {
    match k {
        "VISUAL" => "vi".to_string(),
        _ => String::new(),
    }
}

#[test]
fn brainstorm_opens_the_modal_and_ctrl_j_breaks_the_line() {
    let mut s = opened(Fake::quiet(), Path::new(""));
    type_in(&mut s, "Queue bd writes");
    s.key(ctrl('j'));
    type_in(&mut s, "while offlinex");
    s.key(key(KeyCode::Backspace));
    assert_eq!(text(&s), "Queue bd writes\nwhile offline");
    assert!(s.input.is_empty(), "the keys went to the modal");
    let buf = render(&s, 140, 40);
    let (x, y) = find(&buf, "Queue bd writes").unwrap();
    assert!(
        row(&buf, y + 1).contains("while offline"),
        "{}",
        row(&buf, y + 1)
    );
    assert_eq!(find(&buf, "while offline").unwrap().0, x);
}

#[test]
fn a_paste_keeps_its_newlines() {
    let mut s = opened(Fake::quiet(), Path::new(""));
    s.paste("one\r\ntwo\rthree\n");
    assert_eq!(text(&s), "one\ntwo\nthree\n");
}

#[test]
fn a_paste_on_a_button_presses_nothing() {
    let tools = Fake::quiet();
    let mut s = opened(tools.clone(), Path::new(""));
    type_in(&mut s, "an idea");
    s.key(key(KeyCode::Tab));
    s.paste("more\n");
    assert_eq!(text(&s), "an idea");
    assert!(tools.calls().is_empty(), "{:?}", tools.calls());
}

#[test]
fn a_paste_outside_the_modal_types_as_keys() {
    let mut s = screen_at(Fake::quiet(), Path::new(""));
    s.paste("/brainstorm\n");
    assert!(s.idea.is_some(), "the pasted command ran");
}

#[test]
fn tab_reaches_start_and_cancel_and_esc_or_cancel_close_with_nothing_created() {
    let tools = Fake::quiet();
    let mut s = opened(tools.clone(), Path::new(""));
    type_in(&mut s, "an idea");
    let focus = |s: &Screen| s.idea.as_ref().unwrap().focus;
    s.key(key(KeyCode::Tab));
    assert_eq!(focus(&s), 1, "Start");
    type_in(&mut s, "x");
    assert_eq!(text(&s), "an idea", "a button takes no text");
    s.key(key(KeyCode::Tab));
    assert_eq!(focus(&s), 2, "Cancel");
    s.key(key(KeyCode::Tab));
    assert_eq!(focus(&s), 0, "back on the input");
    s.key(key(KeyCode::BackTab));
    assert_eq!(focus(&s), 2);
    s.key(key(KeyCode::Enter));
    assert!(s.idea.is_none(), "Cancel closed it");

    type_line(&mut s, "/brainstorm");
    type_in(&mut s, "another");
    s.key(key(KeyCode::Esc));
    assert!(s.idea.is_none(), "Esc closed it");
    assert!(tools.calls().is_empty(), "{:?}", tools.calls());
}

#[test]
fn ctrl_g_sets_the_flag_the_run_loop_acts_on() {
    let mut s = opened(Fake::quiet(), Path::new(""));
    s.key(ctrl('g'));
    assert!(s.editing);
    assert!(s.idea.is_some());
}

#[test]
fn the_editor_replaces_the_text_with_one_trailing_newline_stripped() {
    let mut s = opened(Fake::quiet(), Path::new(""));
    type_in(&mut s, "draft");
    let editor = FakeEditor::new(|file| {
        assert_eq!(fs::read_to_string(file).unwrap(), "draft");
        fs::write(file, "first\nsecond\n\n").map_err(|e| e.to_string())
    });
    s.editor = editor.clone();
    s.edit_idea(&visual);
    assert_eq!(text(&s), "first\nsecond\n");
    let calls = editor.calls();
    assert_eq!(calls[0].0, "vi");
    assert!(!calls[0].1.exists(), "the temp file is deleted");
}

#[test]
fn an_empty_file_clears_the_text() {
    let mut s = opened(Fake::quiet(), Path::new(""));
    type_in(&mut s, "draft");
    let editor = FakeEditor::new(|file| fs::write(file, "").map_err(|e| e.to_string()));
    s.editor = editor.clone();
    s.edit_idea(&visual);
    assert_eq!(text(&s), "");
    assert!(!editor.calls()[0].1.exists());
}

#[test]
fn a_non_zero_exit_keeps_the_text_and_warns() {
    let mut s = opened(Fake::quiet(), Path::new(""));
    type_in(&mut s, "draft");
    let editor = FakeEditor::new(|file| {
        fs::write(file, "half").unwrap();
        Err("quit unexpectedly (exit code 1)".to_string())
    });
    s.editor = editor.clone();
    s.edit_idea(&visual);
    assert_eq!(text(&s), "draft");
    assert_eq!(notice(&s), "vi quit unexpectedly (exit code 1)");
    assert!(!editor.calls()[0].1.exists());
}

#[test]
fn start_with_an_empty_input_only_says_so() {
    let tools = Fake::quiet();
    let mut s = opened(tools.clone(), Path::new(""));
    type_in(&mut s, "  ");
    s.key(key(KeyCode::Enter));
    assert!(s.idea.is_some());
    assert_eq!(notice(&s), "write the idea first");
    assert!(tools.calls().is_empty());
}

/// Fake Tools: bd create gives hx-7, `fails` fails.
fn bd(fails: &'static str) -> Arc<Fake> {
    Fake::new(move |_, argv| match argv.join(" ") {
        call if call.starts_with(fails) => Err("boom".to_string()),
        call if call.starts_with("bd create") => Ok("hx-7\n".to_string()),
        call if call.starts_with("git symbolic-ref") => Ok("origin/trunk\n".to_string()),
        _ => Ok(String::new()),
    })
}

#[test]
fn start_creates_the_idea_its_worktree_and_its_state() {
    let repo = TempDir::new();
    let tools = bd("nothing");
    let mut s = opened(tools.clone(), repo.path());
    type_in(&mut s, "Queue bd writes");
    s.key(ctrl('j'));
    type_in(&mut s, "while offline");
    s.key(key(KeyCode::Tab));
    s.key(key(KeyCode::Enter));
    let worktree = repo.path().join(LOCAL).join("worktrees/hx-7");
    assert_eq!(
        tools.calls(),
        [
            "bd create --type=task --labels=brainstorm:idea --title=Queue bd writes \
             --description=Queue bd writes\nwhile offline --silent"
                .to_string(),
            "bd update hx-7 --status in_progress".to_string(),
            "git symbolic-ref --short refs/remotes/origin/HEAD".to_string(),
            format!(
                "git worktree add -b brainstorm/hx-7 {} origin/trunk",
                worktree.display()
            ),
        ]
    );
    let saved = repo.path().join(LOCAL).join("brainstorms/hx-7/state.json");
    let saved: Brainstorm = serde_json::from_slice(&fs::read(saved).unwrap()).unwrap();
    let want = Brainstorm {
        phase: Phase::Charting,
        idea: "hx-7".to_string(),
        worktree: worktree.display().to_string(),
        branch: "brainstorm/hx-7".to_string(),
        ..Default::default()
    };
    assert_eq!(saved, want);
    assert_eq!(s.brainstorms, [want]);
    assert!(s.idea.is_none(), "the modal closed");
    assert_eq!(
        line(s.events.last().unwrap()),
        "hx-7 Idea created from your text; worktree on brainstorm/hx-7"
    );
}

#[test]
fn start_keeps_the_whole_text_as_the_description() {
    let repo = TempDir::new();
    let tools = bd("nothing");
    let mut s = opened(tools.clone(), repo.path());
    s.paste("\n  Queue bd writes\n\n    while offline\n");
    s.key(key(KeyCode::Tab));
    s.key(key(KeyCode::Enter));
    assert_eq!(
        tools.calls()[0],
        "bd create --type=task --labels=brainstorm:idea --title=Queue bd writes \
         --description=\n  Queue bd writes\n\n    while offline\n --silent"
    );
}

#[test]
fn a_failed_bd_create_keeps_the_modal_and_its_text() {
    let repo = TempDir::new();
    let mut s = opened(bd("bd create"), repo.path());
    type_in(&mut s, "an idea");
    s.key(key(KeyCode::Enter));
    assert_eq!(text(&s), "an idea");
    assert!(line(s.events.last().unwrap()).starts_with("Idea not created: bd create"));
}

#[test]
fn a_bd_create_with_no_id_keeps_the_modal_and_creates_nothing() {
    let tools = Fake::quiet();
    let mut s = opened(tools.clone(), TempDir::new().path());
    type_in(&mut s, "an idea");
    s.key(key(KeyCode::Enter));
    assert_eq!(text(&s), "an idea");
    assert_eq!(tools.calls().len(), 1, "only bd create ran");
    assert_eq!(
        line(s.events.last().unwrap()),
        "Idea not created: bd create gave no id"
    );
}

#[test]
fn a_failed_worktree_says_so_and_saves_no_state() {
    let repo = TempDir::new();
    let mut s = opened(bd("git worktree add"), repo.path());
    type_in(&mut s, "an idea");
    s.key(key(KeyCode::Enter));
    assert!(s.idea.is_none(), "bd holds the text");
    let said = line(s.events.last().unwrap());
    assert!(
        said.starts_with("hx-7 worktree not created: git worktree add"),
        "{said}"
    );
    assert!(!repo.path().join(LOCAL).join("brainstorms").exists());
    assert!(s.brainstorms.is_empty());
}

#[test]
fn the_modal_docks_beside_the_shell_at_140_columns() {
    let mut s = opened(Fake::quiet(), Path::new(""));
    type_in(&mut s, "an idea");
    s.key(key(KeyCode::Tab));
    let buf = render(&s, 140, 40);
    let (x, _) = find(&buf, " BRAINSTORM · a new idea ").unwrap();
    assert!(x >= 58, "the Shell keeps the left 42%: at {x}");
    for want in [
        "What is the idea? Paste or write it here",
        "an idea",
        "Ctrl+G opens it in your editor ($VISUAL, $EDITOR, else code -w, vi, nano)",
        "Cancel",
        " Enter starts · Tab moves · Ctrl+J a new line · Esc cancels ",
        "Start creates the Idea in bd and its worktree on brainstorm/<idea>.",
    ] {
        assert!(find(&buf, want).is_some(), "no {want:?}");
    }
    let (x, y) = find(&buf, "› Start").unwrap();
    assert_eq!(buf[(x, y)].bg, GREEN, "the focused button is bright");
    // the input's rounded box, ten rows tall
    let (_, top) = find(&buf, "What is the idea?").unwrap();
    assert!(row(&buf, top + 2).contains('╭'), "{}", row(&buf, top + 2));
    assert!(row(&buf, top + 11).contains('╰'), "{}", row(&buf, top + 11));
}

#[test]
fn the_modal_folds_over_the_shell_under_110_columns() {
    let mut s = opened(Fake::quiet(), Path::new(""));
    type_in(&mut s, "an idea");
    let buf = render(&s, 90, 40);
    let (x, _) = find(&buf, " BRAINSTORM · a new idea ").unwrap();
    assert!(x < 20, "folded over the Shell: at {x}");
    for want in [
        "What is the idea? Paste or write it here",
        "an idea▌",
        "Start",
        "Cancel",
    ] {
        assert!(find(&buf, want).is_some(), "no {want:?}");
    }
    // a line longer than the box wraps onto the next row
    type_in(&mut s, &" word".repeat(20));
    let buf = render(&s, 90, 40);
    let (_, y) = find(&buf, "an idea word").unwrap();
    assert!(
        row(&buf, y + 1).contains("word word"),
        "{}",
        row(&buf, y + 1)
    );
    assert!(row(&buf, y + 1).contains('▌'), "{}", row(&buf, y + 1));
}
