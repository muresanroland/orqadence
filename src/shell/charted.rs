//! What charting came out as: the Tickets modal, which starts the checked
//! Tickets with /start-ticket, and the start-Map modal, which makes the Map
//! the live Brainstorm with research in the background or not; its
//! Continue form asks again with the answer saved last time.

use crossterm::event::{KeyCode, KeyEvent};
use std::sync::atomic::Ordering;

use super::{About, Screen};
use crate::brainstorm::{driver, labelled, research, Brainstorm, Phase, EPIC, RESEARCH};
use crate::orchestrator::app::{self, MAX_RESEARCH};
use crate::orchestrator::scheduler::BdIssue;
use crate::orchestrator::stage::{plural, Ask, Config};

/// One Ticket of the Tickets modal.
pub(crate) struct Row {
    pub(crate) id: String,
    pub(crate) title: String,
    /// Its description's first line.
    pub(crate) about: String,
    pub(crate) on: bool,
}

/// The Tickets modal: focus on a row, then Start tickets, then Cancel.
pub(crate) struct Tickets {
    pub(crate) idea: String,
    pub(crate) title: String,
    pub(crate) rows: Vec<Row>,
    pub(crate) focus: usize,
}

impl Tickets {
    /// What Start tickets runs: /start-ticket on the checked ones.
    pub(crate) fn command(&self) -> String {
        let checked = self.rows.iter().filter(|r| r.on);
        let ids: Vec<String> = checked.map(|r| format!("@{}", r.id)).collect();
        format!("/start-ticket {}", ids.join(" "))
    }
}

/// The start-Map modal, or its Continue form: focus on the checkbox, then
/// Start Map (Continue), then Cancel.
pub(crate) struct StartMap {
    pub(crate) idea: String,
    pub(crate) map: String,
    pub(crate) title: String,
    /// The Map's ## Destination section, on one line.
    pub(crate) destination: String,
    /// Its Waypoints counted, for the form it shows.
    pub(crate) counts: String,
    /// Its open Research Waypoints.
    pub(crate) research: usize,
    pub(crate) max_research: usize,
    /// The Continue form, on /continue @<map>: the answer is the saved one.
    pub(crate) again: bool,
    /// The answer: research starts in the background.
    pub(crate) background: bool,
    /// The Continue form's rebase line, empty until one is tried.
    pub(crate) rebase: String,
    pub(crate) focus: usize,
}

/// The lines under a description's "## Destination" heading, up to the
/// next heading, on one line.
fn destination(description: &str) -> String {
    let lines = description
        .lines()
        .skip_while(|l| l.trim() != "## Destination");
    let lines = lines.skip(1).take_while(|l| !l.starts_with('#'));
    let lines: Vec<&str> = lines.map(str::trim).filter(|l| !l.is_empty()).collect();
    lines.join(" ")
}

impl Screen {
    /// An Epic run is live: /start-ticket joins only a Ticket run.
    pub(crate) fn epic_live(&self) -> bool {
        self.run.as_ref().is_some_and(|r| r.epic)
    }

    /// Start tickets can be pressed: a Ticket checked, no Epic run live.
    pub(crate) fn tickets_start(&self) -> bool {
        let checked = |t: &Tickets| t.rows.iter().any(|r| r.on);
        !self.epic_live() && self.tickets.as_ref().is_some_and(checked)
    }

    /// The Tickets modal's keys: ↑↓ move over the rows and the buttons,
    /// Space checks a row, Tab goes to Start tickets, then Cancel, then
    /// the first row (Shift-Tab back), Enter presses the focused button,
    /// Esc cancels. Cancel leaves the Tickets open in bd.
    pub(super) fn tickets_key(&mut self, key: KeyEvent) {
        let start = self.tickets_start();
        let Some(t) = &mut self.tickets else {
            return;
        };
        let (n, f) = (t.rows.len(), t.focus);
        match key.code {
            KeyCode::Up => t.focus = f.saturating_sub(1),
            KeyCode::Down => t.focus = (f + 1).min(n + 1),
            KeyCode::Tab if f < n => t.focus = n,
            KeyCode::Tab => t.focus = (f + 1) % (n + 2),
            KeyCode::BackTab if f == n + 1 => t.focus = n,
            KeyCode::BackTab if f == n => t.focus = 0,
            KeyCode::BackTab => t.focus = n + 1,
            KeyCode::Char(' ') if f < n => t.rows[f].on ^= true,
            KeyCode::Enter if f == n && start => {
                let command = t.command();
                self.tickets = None;
                self.command(&command);
            }
            KeyCode::Enter if f == n + 1 => self.tickets = None,
            KeyCode::Esc => self.tickets = None,
            _ => {}
        }
    }

    /// The start-Map modal's keys: Tab and Shift-Tab move the focus, Space
    /// flips the checkbox, Enter on the checkbox or the green button starts
    /// the Map, Enter on Cancel or Esc cancels.
    pub(super) fn start_map_key(&mut self, key: KeyEvent) {
        let Some(m) = &mut self.start_map else {
            return;
        };
        match key.code {
            KeyCode::Tab => m.focus = (m.focus + 1) % 3,
            KeyCode::BackTab => m.focus = (m.focus + 2) % 3,
            KeyCode::Char(' ') => m.background ^= true,
            KeyCode::Enter if m.focus < 2 => self.start_the_map(),
            KeyCode::Enter | KeyCode::Esc => self.cancel_map(),
            _ => {}
        }
    }

    /// Start Map, or Continue: the answer in the Brainstorm's state, and
    /// the Map the live Brainstorm, its session with you started.
    fn start_the_map(&mut self) {
        let Some(m) = self.start_map.take() else {
            return;
        };
        // its driver still running owns its pane, result file and state
        if self.driving(&m.idea) {
            let text = format!("refused: a Waypoint session of {} is running", m.map);
            return self.refuse(&text);
        }
        let Some(b) = self.brainstorms.iter_mut().find(|b| b.idea == m.idea) else {
            return;
        };
        let previous = (b.background, b.started);
        (b.background, b.started) = (m.background, true);
        if let Err(err) = b.save(&self.cfg.repo) {
            (b.background, b.started) = previous;
            self.tell(Some(&m.map), &format!("Brainstorm state not saved: {err}"));
            self.start_map = Some(m);
            return;
        }
        let b = b.clone();
        self.live = Some(m.idea);
        let text = match m.background {
            true => "live: research in the background",
            false => "live: research with you",
        };
        self.tell(Some(&m.map), text);
        self.work_map(b, None, None);
    }

    /// A session with the user on the live Map of `b`, on a driver thread
    /// of its own: its PROMPT `prompt`, its Waypoint `pick` when named. A
    /// Next Waypoint? still waiting goes; never outside herdr, with no
    /// Shell's pane to split.
    pub(super) fn work_map(&mut self, b: Brainstorm, prompt: Option<String>, pick: Option<String>) {
        self.questions
            .retain(|q| !matches!(q.about, About::Asked(Ask::NextWaypoint { .. })));
        self.drive(b, "Waypoint", move |cfg, shell, b, saved, stop| {
            driver::waypoint(cfg, shell, b, saved, stop, prompt, pick);
        });
    }

    /// The research threads' changes saved with their Brainstorms; each
    /// told whether its Map is live with research in the background; one
    /// started for the Map that is, or that holds research to take up; and
    /// after a research close, with no session with you running, Next
    /// Waypoint? asked when the frontier gives one.
    pub(super) fn research_poll(&mut self) {
        // the ended ones first: what they sent before they ended is read
        // below, so none is started again over a stale list
        self.research.retain(|r| !r.thread.is_finished());
        while let Ok(u) = self.research_receiver.try_recv() {
            if let Some(b) = self.brainstorms.iter_mut().find(|b| b.idea == u.idea) {
                (b.research_tab, b.research) = (u.tab, u.research);
                if let Err(err) = b.save(&self.cfg.repo) {
                    let (key, text) = (
                        b.key().to_string(),
                        format!("Brainstorm state not saved: {err}"),
                    );
                    self.tell(Some(&key), &text);
                }
            }
            if let Some(id) = u.closed {
                self.research_closed = Some((u.idea, id));
            }
        }
        let live = |s: &Self, b: &Brainstorm| s.live.as_ref() == Some(&b.idea) && b.background;
        for r in &self.research {
            let b = self.brainstorms.iter().find(|b| b.idea == r.idea);
            r.live
                .store(b.is_some_and(|b| live(self, b)), Ordering::SeqCst);
        }
        let wanted: Vec<Brainstorm> = (self.brainstorms.iter())
            .filter(|b| b.phase == Phase::Map && !self.research.iter().any(|r| r.idea == b.idea))
            .filter(|b| live(self, b) || b.research.iter().any(|r| !r.parked))
            .cloned()
            .collect();
        for b in wanted {
            let on = live(self, &b);
            let (stop, sender) = (self.brainstorm_stop.clone(), self.research_sender.clone());
            let cfg = Config {
                events: self.sender.clone(),
                ..self.cfg.clone()
            };
            self.research
                .push(research::spawn(&cfg, b, on, stop, sender));
        }
        let Some((idea, id)) = self.research_closed.clone() else {
            return;
        };
        let live = self
            .live_brainstorm()
            .filter(|b| b.idea == idea && b.phase == Phase::Map);
        let Some(b) = live.cloned() else {
            self.research_closed = None;
            return;
        };
        if self.driving(&idea) {
            return; // its session's own end asks, or it is still starting
        }
        self.research_closed = None;
        let asked = |q: &super::Question| matches!(&q.about, About::Asked(Ask::NextWaypoint { map, .. }) if *map == b.map);
        if b.pane.is_empty() && b.session.is_none() && !self.questions.iter().any(asked) {
            self.drive(b, "Waypoint", move |cfg, _, b, saved, stop| {
                driver::research_closed(cfg, b, saved, stop, &id);
            });
        }
    }

    /// Cancel keeps the Map; the start form leaves /continue @<map> as the
    /// suggestion.
    fn cancel_map(&mut self) {
        let Some(m) = self.start_map.take() else {
            return;
        };
        if !m.again {
            let next = format!("/continue @{}", m.map);
            self.tell(Some(&m.map), &format!("not started: {next} starts it"));
            self.suggestion = Some(next);
        }
    }

    /// `bd show` of `ids`, in one call; nothing and a RECENT line when bd fails.
    fn shown(&mut self, ids: &[&str]) -> Vec<BdIssue> {
        let mut argv = vec!["bd", "show"];
        argv.extend(ids);
        argv.push("--json");
        let out = self.cfg.tools.run(&self.cfg.repo, &argv);
        match out.map_err(|e| e.to_string()).and_then(|out| {
            serde_json::from_str::<Option<Vec<BdIssue>>>(&out).map_err(|e| e.to_string())
        }) {
            Ok(issues) => issues.unwrap_or_default(),
            Err(err) => {
                self.say(&format!("bd show failed: {err}"));
                Vec::new()
            }
        }
    }

    /// The next of charting's outcomes, once neither modal is open.
    pub(super) fn open_charted(&mut self) {
        while self.tickets.is_none() && self.start_map.is_none() {
            let Some(b) = self.charted.pop_front() else {
                return;
            };
            match b.phase {
                Phase::Map => self.open_start_map(&b, false),
                _ => self.open_tickets(&b),
            }
        }
    }

    /// The Tickets modal for the Tickets `b` charted, each checked, with
    /// its title and its description's first line as bd shows them.
    pub(crate) fn open_tickets(&mut self, b: &Brainstorm) {
        let mut ids = vec![b.idea.as_str()];
        ids.extend(b.tickets.iter().map(String::as_str));
        let shown = self.shown(&ids);
        let issue = |id: &str| shown.iter().find(|i| i.id == id);
        let rows = b
            .tickets
            .iter()
            .map(|id| {
                let i = issue(id);
                let about = i.and_then(|i| i.description.lines().find(|l| !l.trim().is_empty()));
                Row {
                    id: id.clone(),
                    title: i.map(|i| i.title.clone()).unwrap_or_default(),
                    about: about.unwrap_or_default().trim().to_string(),
                    on: true,
                }
            })
            .collect();
        self.tickets = Some(Tickets {
            idea: b.idea.clone(),
            title: issue(&b.idea).map(|i| i.title.clone()).unwrap_or_default(),
            rows,
            focus: 0,
        });
    }

    /// The start-Map modal for `b`'s Map, its answer research in the
    /// background; `again`, its Continue form, at the answer `b` saved.
    pub(crate) fn open_start_map(&mut self, b: &Brainstorm, again: bool) {
        let map = self.shown(&[&b.map]).into_iter().next().unwrap_or_default();
        let issues = self.reload_issues().unwrap_or_default();
        let waypoints: Vec<&BdIssue> = issues.iter().filter(|i| i.parent == b.map).collect();
        let open = |i: &&&BdIssue| i.status != "closed";
        let research = waypoints.iter().filter(|i| labelled(i, RESEARCH));
        let epic = waypoints.iter().filter(|i| labelled(i, EPIC)).count();
        let with_you = waypoints
            .iter()
            .filter(|i| !labelled(i, RESEARCH) && !labelled(i, EPIC));
        let counts = if again {
            let closed = waypoints.len() - waypoints.iter().filter(open).count();
            format!(
                "{closed}/{} closed · {} with you open · {} research open",
                plural(waypoints.len(), "Waypoint"),
                with_you.filter(open).count(),
                research.clone().filter(open).count(),
            )
        } else {
            format!(
                "{}: {} with you, {} research, {epic} writes the Epic · branch {}",
                plural(waypoints.len(), "Waypoint"),
                with_you.count(),
                research.clone().count(),
                b.branch,
            )
        };
        self.start_map = Some(StartMap {
            idea: b.idea.clone(),
            map: b.map.clone(),
            title: map.title,
            destination: destination(&map.description),
            counts,
            research: research.filter(open).count(),
            max_research: app::count(&self.cfg.repo, &MAX_RESEARCH),
            again,
            background: !again || b.background,
            rebase: String::new(),
            focus: 0,
        });
    }
}
