//! /brainstorm's idea modal: the text typed, pasted or written in the
//! user's editor with Ctrl+G, and Start, which creates the Idea in bd, its
//! worktree on brainstorm/<idea> and the Brainstorm's state file, then the
//! charting session on its driver's thread.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{Screen, NOTICE_WINDOW};
use crate::brainstorm::{driver, Brainstorm, IDEA};
use crate::graphify;
use crate::orchestrator::pipeline::origin_head;
use crate::orchestrator::stage::{worktree, Config};
use crate::tools;

/// The idea modal's text and focus: 0 the input, 1 Start, 2 Cancel.
#[derive(Default)]
pub(crate) struct Idea {
    pub(crate) text: String,
    pub(crate) focus: usize,
}

impl Screen {
    /// The idea modal's keys: Tab and Shift-Tab move the focus; on the
    /// input a key types, Backspace deletes, Ctrl+J breaks the line and
    /// Enter starts; Enter on a button presses it; Esc cancels; Ctrl+G sets
    /// the flag the run loop opens the editor on.
    pub(super) fn idea_key(&mut self, key: KeyEvent) {
        let Some(idea) = &mut self.idea else {
            return;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Tab => idea.focus = (idea.focus + 1) % 3,
            KeyCode::BackTab => idea.focus = (idea.focus + 2) % 3,
            KeyCode::Esc => self.idea = None,
            KeyCode::Enter if idea.focus == 2 => self.idea = None,
            KeyCode::Enter => self.start_idea(),
            KeyCode::Char('g') if ctrl => self.editing = true,
            KeyCode::Char('j') if ctrl && idea.focus == 0 => idea.text.push('\n'),
            KeyCode::Char(c) if !ctrl && idea.focus == 0 => idea.text.push(c),
            KeyCode::Backspace if idea.focus == 0 => {
                idea.text.pop();
            }
            _ => {}
        }
    }

    /// A bracketed paste: into the idea modal's input with its newlines,
    /// never pressing its buttons; anywhere else typed key by key, as it
    /// came before the Shell asked for bracketed paste.
    pub(crate) fn paste(&mut self, text: &str) {
        match &mut self.idea {
            Some(idea) if idea.focus == 0 => {
                idea.text += &text.replace("\r\n", "\n").replace('\r', "\n")
            }
            Some(_) => {}
            None => {
                for c in text.chars() {
                    let code = match c {
                        '\n' | '\r' => KeyCode::Enter,
                        '\t' => KeyCode::Tab,
                        c => KeyCode::Char(c),
                    };
                    self.key(KeyEvent::new(code, KeyModifiers::NONE));
                }
            }
        }
    }

    /// Ctrl+G: the idea's text in the user's editor, through a temp file
    /// deleted whatever happens. A non-zero exit or a signal keeps the text
    /// and warns; otherwise the file's text replaces it, one trailing
    /// newline stripped, so an empty file clears it. The run loop has
    /// handed the terminal over.
    pub(crate) fn edit_idea(&mut self, env: &dyn Fn(&str) -> String) {
        let Some(text) = self.idea.as_ref().map(|i| i.text.clone()) else {
            return;
        };
        let Some(editor) = tools::editor(env) else {
            return self.notice("no editor: set $VISUAL or $EDITOR", NOTICE_WINDOW);
        };
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let name = format!("orqa-idea-{}-{nanos}.md", std::process::id());
        let file = std::env::temp_dir().join(name);
        let written = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file)
            .and_then(|mut f| f.write_all(text.as_bytes()));
        let edited = match written {
            Ok(()) => self.editor.edit(&editor, &file),
            Err(err) => Err(format!("not opened: {err}")),
        };
        let read = fs::read_to_string(&file);
        let _ = fs::remove_file(&file);
        let program = editor.split(' ').next().unwrap_or(&editor);
        match (edited, read) {
            (Err(why), _) => self.notice(&format!("{program} {why}"), NOTICE_WINDOW),
            (Ok(()), Err(err)) => self.notice(&format!("{program}: {err}"), NOTICE_WINDOW),
            (Ok(()), Ok(new)) => {
                if let Some(idea) = &mut self.idea {
                    idea.text = new.strip_suffix('\n').unwrap_or(&new).to_string();
                }
            }
        }
    }

    /// Start: the Idea in bd, in progress, titled by the text's first line;
    /// its worktree on a new branch brainstorm/<idea> from origin's default
    /// branch, with its code graph; then its state file, charting. A failed
    /// step says so on RECENT and stops, leaving what bd or git already
    /// hold. The modal closes once bd holds the text.
    fn start_idea(&mut self) {
        let Some(text) = self.idea.as_ref().map(|i| i.text.clone()) else {
            return;
        };
        if text.trim().is_empty() {
            return self.notice("write the idea first", NOTICE_WINDOW);
        }
        let (tools, repo) = (self.cfg.tools.clone(), self.cfg.repo.clone());
        let first = text.trim().lines().next().unwrap_or_default();
        let title = format!("--title={first}");
        let labels = format!("--labels={IDEA}");
        let description = format!("--description={text}");
        let create = [
            "bd",
            "create",
            "--type=task",
            &labels,
            &title,
            &description,
            "--silent",
        ];
        let id = match tools.run(&repo, &create) {
            Ok(id) if !id.trim().is_empty() => id.trim().to_string(),
            Ok(_) => return self.say("Idea not created: bd create gave no id"),
            Err(err) => return self.say(&format!("Idea not created: {err}")),
        };
        self.idea = None;
        let tell = |s: &mut Screen, text: String| s.tell(Some(&id), &text);
        if let Err(err) = tools.run(&repo, &["bd", "update", &id, "--status", "in_progress"]) {
            return tell(self, format!("not marked in_progress: {err}"));
        }
        let base = origin_head(&*tools, &repo);
        let (path, branch) = (worktree(&repo, &id), format!("brainstorm/{id}"));
        let shown = path.display().to_string();
        let add = ["git", "worktree", "add", "-b", &branch, &shown, &base];
        if let Err(err) = tools.run(&repo, &add) {
            return tell(self, format!("worktree not created: {err}"));
        }
        // ponytail: graphify update runs on the Shell's thread, which stalls
        // the screen up to its limit; a thread if that shows
        if let Err(err) = graphify::prepare_worktree(&*tools, &repo, &path) {
            tell(self, format!("code graph not built: {err}"));
        }
        let b = Brainstorm {
            idea: id.clone(),
            worktree: shown,
            branch: branch.clone(),
            ..Default::default()
        };
        if let Err(err) = b.save(&repo) {
            return tell(self, format!("Brainstorm state not saved: {err}"));
        }
        self.brainstorms.push(b.clone());
        tell(
            self,
            format!("Idea created from your text; worktree on {branch}"),
        );
        self.chart(b);
    }

    /// The charting session for `b`, the live Brainstorm.
    pub(super) fn chart(&mut self, b: Brainstorm) {
        self.drive(b, "charting", driver::chart);
    }

    /// `run` for `b` on a driver thread of its own, `b` the live
    /// Brainstorm: never outside herdr, with no Shell's pane to split, the
    /// `noun` not started.
    pub(super) fn drive(
        &mut self,
        b: Brainstorm,
        noun: &str,
        run: impl FnOnce(&Config, &str, Brainstorm, &Sender<Brainstorm>, &AtomicBool) + Send + 'static,
    ) {
        if self.shell_pane.is_empty() {
            let text =
                format!("{noun} not started: the Shell is not in a herdr pane (HERDR_PANE_ID)");
            return self.tell(Some(b.key()), &text);
        }
        let cfg = Config {
            events: self.sender.clone(),
            ..self.cfg.clone()
        };
        let (shell, saved, stop) = (
            self.shell_pane.clone(),
            self.brainstorm_sender.clone(),
            self.brainstorm_stop.clone(),
        );
        let idea = b.idea.clone();
        let driver = thread::spawn(move || run(&cfg, &shell, b, &saved, &stop));
        self.brainstorm_threads.retain(|(_, t)| !t.is_finished());
        self.brainstorm_threads.push((idea.clone(), driver));
        self.live = Some(idea);
    }
}
