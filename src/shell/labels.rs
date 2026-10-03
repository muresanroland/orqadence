//! The proposed-label modal: the LABEL lines of a brainstorm-chart or
//! brainstorm-epic result, a row each. Per row the user accepts the label,
//! picks a configured one in its place, or none; only Apply writes
//! config.json and bd, never the session. Cancel leaves the lines in the
//! Brainstorm's state, for /continue @<id>.

use crossterm::event::{KeyCode, KeyEvent};
use serde_json::json;

use super::config::valid_label;
use super::Screen;
use crate::brainstorm::Brainstorm;
use crate::orchestrator::app;
use crate::skills::manifest::{self, parse_source, Added, Manifest, PREFIX};

/// A row's answer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Choice {
    /// The label written to config.json, then put on its Tickets.
    Accept,
    /// This configured label put on its Tickets instead.
    Existing(String),
    /// Nothing changes.
    Skip,
}

/// One LABEL line: `<name> | <guidance> | skills: <a>, <b> | tickets: <ids>`.
pub(crate) struct Row {
    /// The line as the Brainstorm's state keeps it.
    pub(crate) line: String,
    /// Bare: docs, not orqa:docs.
    pub(crate) name: String,
    pub(crate) guidance: String,
    /// Installed names or sources.
    pub(crate) skills: Vec<String>,
    pub(crate) tickets: Vec<String>,
    pub(crate) choice: Choice,
}

/// The line read field by field; a field missing is empty.
fn row(line: &str) -> Row {
    let mut parts = line.split('|').map(str::trim);
    let mut next = || parts.next().unwrap_or_default();
    let name = next();
    let guidance = next().to_string();
    let list = |part: &str, key: &str| -> Vec<String> {
        let part = part.strip_prefix(key).unwrap_or(part);
        let items = part.split([',', ' ']).filter(|s| !s.is_empty());
        items.map(String::from).collect()
    };
    Row {
        line: line.to_string(),
        name: name.strip_prefix("orqa:").unwrap_or(name).to_string(),
        guidance,
        skills: list(next(), "skills:"),
        tickets: list(next(), "tickets:"),
        choice: Choice::Accept,
    }
}

/// The modal: focus on a row, then Apply, then Cancel.
pub(crate) struct Labels {
    /// The Brainstorm's Idea, and what RECENT calls it: its Map once
    /// there is one.
    pub(crate) idea: String,
    pub(crate) key: String,
    pub(crate) rows: Vec<Row>,
    /// config.json's labels as the modal opened, the picker's choices.
    pub(crate) configured: Vec<String>,
    pub(crate) focus: usize,
    /// The outcome it stands before opens once it closes.
    pub(crate) outcome: bool,
}

impl Labels {
    /// A row's answers in the picker's order: accept, each configured
    /// label, none.
    fn choices(&self) -> Vec<Choice> {
        let existing = self.configured.iter().cloned().map(Choice::Existing);
        let mut choices = vec![Choice::Accept];
        choices.extend(existing);
        choices.push(Choice::Skip);
        choices
    }

    /// The focused row's answer moved `by` along the picker, round.
    fn pick(&mut self, by: usize) {
        let choices = self.choices();
        let Some(r) = self.rows.get_mut(self.focus) else {
            return;
        };
        let at = choices.iter().position(|c| *c == r.choice).unwrap_or(0);
        r.choice = choices[(at + by) % choices.len()].clone();
    }
}

impl Screen {
    /// The modal on `b`'s LABEL lines, each row at accept; `outcome`, it
    /// stands before charting's outcome or the Map's end.
    pub(super) fn open_labels(&mut self, b: &Brainstorm, outcome: bool) {
        let configured = app::read_object(&self.cfg.repo)
            .map(|(_, doc)| app::labels(&doc).into_keys().collect())
            .unwrap_or_default();
        self.labels = Some(Labels {
            idea: b.idea.clone(),
            key: b.key().to_string(),
            rows: b.label_lines.iter().map(|l| row(l)).collect(),
            configured,
            focus: 0,
            outcome,
        });
    }

    /// The modal's keys: ↑↓ move over the rows and the buttons, ←→ pick a
    /// row's answer, Tab goes to Apply, then Cancel, then the first row
    /// (Shift-Tab back), Enter presses the focused button, Esc cancels.
    pub(super) fn labels_key(&mut self, key: KeyEvent) {
        let Some(l) = &mut self.labels else {
            return;
        };
        let (n, f) = (l.rows.len(), l.focus);
        let all = l.choices().len();
        match key.code {
            KeyCode::Up => l.focus = f.saturating_sub(1),
            KeyCode::Down => l.focus = (f + 1).min(n + 1),
            KeyCode::Right if f < n => l.pick(1),
            KeyCode::Left if f < n => l.pick(all - 1),
            KeyCode::Tab if f < n => l.focus = n,
            KeyCode::Tab => l.focus = (f + 1) % (n + 2),
            KeyCode::BackTab if f == n + 1 => l.focus = n,
            KeyCode::BackTab if f == n => l.focus = 0,
            KeyCode::BackTab => l.focus = n + 1,
            KeyCode::Enter if f == n => self.apply_labels(),
            KeyCode::Enter if f == n + 1 => self.close_labels(),
            KeyCode::Esc => self.close_labels(),
            _ => {}
        }
    }

    /// Closed: the outcome it stood before, as the Brainstorm is now.
    fn close_labels(&mut self) {
        let Some(l) = self.labels.take() else {
            return;
        };
        let b = self.brainstorms.iter().find(|b| b.idea == l.idea).cloned();
        if let (true, Some(b)) = (l.outcome, b) {
            self.open_outcome(&b);
        }
    }

    /// Apply: each row answered, said under the Brainstorm's key. A row
    /// that failed keeps its line in the Brainstorm's state; the rest are
    /// gone from it.
    fn apply_labels(&mut self) {
        let Some(l) = &self.labels else {
            return;
        };
        let (idea, key) = (l.idea.clone(), l.key.clone());
        let rows: Vec<(String, Result<String, String>)> = l
            .rows
            .iter()
            .map(|r| (r.line.clone(), self.answer_label(r)))
            .collect();
        let mut left = Vec::new();
        for (line, said) in rows {
            let text = said.unwrap_or_else(|err| {
                left.push(line);
                err
            });
            self.tell(Some(&key), &text);
        }
        if let Some(b) = self.brainstorms.iter_mut().find(|b| b.idea == idea) {
            b.label_lines = left;
            if let Err(err) = b.save(&self.cfg.repo) {
                self.tell(Some(&key), &format!("Brainstorm state not saved: {err}"));
            }
        }
        self.close_labels();
    }

    /// What `r`'s answer does, said; Err says why it did not.
    fn answer_label(&self, r: &Row) -> Result<String, String> {
        let name = &r.name;
        fn not(label: &str) -> impl Fn(String) -> String + '_ {
            move |err| format!("orqa:{label} not added: {err}")
        }
        match &r.choice {
            Choice::Skip => Ok(format!("orqa:{name} not added")),
            Choice::Existing(other) => {
                let on = self.label_tickets(other, &r.tickets).map_err(not(other))?;
                Ok(format!("orqa:{other} in place of orqa:{name}{on}"))
            }
            Choice::Accept => {
                let wrote = self.write_label(r).map_err(not(name))?;
                let on = self.label_tickets(name, &r.tickets).map_err(not(name))?;
                Ok(format!("orqa:{name} accepted: {wrote}{on}"))
            }
        }
    }

    /// `r` written to config.json as an area label with its guidance and
    /// the skills it could have; one there already is kept as it is. What
    /// it did, said.
    fn write_label(&self, r: &Row) -> Result<String, String> {
        let (path, mut doc) = app::read_object(&self.cfg.repo)?;
        if doc["labels"].get(&r.name).is_some() {
            return Ok("config.json has it already".to_string());
        }
        let name = valid_label(&doc, &r.name)?;
        if doc["labels"].is_null() {
            doc["labels"] = json!({});
        }
        let labels = doc["labels"]
            .as_object_mut()
            .ok_or_else(|| "labels in config.json is not an object".to_string())?;
        let (skills, off) = self.label_skills(&r.skills);
        let mut said = "an area label in config.json".to_string();
        if !skills.is_empty() {
            said += &format!(", skills {}", skills.join(", "));
        }
        let entry = json!({"kind": "area", "guidance": r.guidance, "skills": skills});
        labels.insert(name, entry);
        app::write(&path, &doc)?;
        Ok(off.iter().fold(said, |said, off| format!("{said}; {off}")))
    }

    /// `skills` by their installed names: one in the Skill manifest as it
    /// is, one given as a source installed as /config's label page installs
    /// it. Those it could not have, each with why.
    // ponytail: the clone runs on the Shell's thread, which stalls the
    // screen that long; off the thread, as /config does, if that is felt.
    fn label_skills(&self, skills: &[String]) -> (Vec<String>, Vec<String>) {
        let (repo, tools) = (&self.cfg.repo, &*self.cfg.tools);
        let mut on = Vec::new();
        let mut off = Vec::new();
        for skill in skills {
            let names = [skill.clone(), format!("{PREFIX}{skill}")];
            let had = Manifest::load(repo)
                .ok()
                .and_then(|m| names.into_iter().find(|n| m.skills.contains_key(n)));
            let got = match had {
                Some(name) => Ok(name),
                None if parse_source(skill).is_err() => Err("not installed".to_string()),
                None => match manifest::add(repo, tools, skill, None) {
                    Ok(Added::Installed(name)) => Ok(name),
                    Ok(Added::Choose(names)) => Err(format!("it holds {}", names.join(", "))),
                    Err(err) => Err(err),
                },
            };
            match got {
                Ok(name) => on.push(name),
                Err(err) => off.push(format!("{skill} left off: {err}")),
            }
        }
        (on, off)
    }

    /// orqa:<name> on each of `tickets` in bd; said as "; on <tickets>".
    fn label_tickets(&self, name: &str, tickets: &[String]) -> Result<String, String> {
        let label = format!("orqa:{name}");
        for t in tickets {
            let argv = ["bd", "label", "add", t, &label];
            self.cfg
                .tools
                .run(&self.cfg.repo, &argv)
                .map_err(|err| format!("bd label add {t} failed: {err}"))?;
        }
        Ok(match tickets.is_empty() {
            true => String::new(),
            false => format!("; on {}", tickets.join(", ")),
        })
    }
}
