//! A Brainstorm's research in the background: every unclaimed Research
//! Waypoint on its live Map's frontier, max_research at once, each the
//! RESEARCH Stage of the Brainstorm's own Orchestrator in tab
//! research-<map>, so it has a Stage's Wake, Questions and Away. One thread
//! per Brainstorm owns its research list; each change goes back to the
//! Shell, which saves it with the Brainstorm.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use super::{labelled, Brainstorm, Research, RESEARCH as LABEL};
use crate::orchestrator::app::{self, MAX_RESEARCH};
use crate::orchestrator::herdr;
use crate::orchestrator::result::{ResultRequirements, StageResult};
use crate::orchestrator::stage::{
    log_file, result_name, Config, Orchestrator, StageError, AWAY, RESEARCH,
};
use crate::shell::suffix;

/// A change of a Brainstorm's research, for the Shell to save.
pub(crate) struct Update {
    pub(crate) idea: String,
    pub(crate) tab: String,
    pub(crate) research: Vec<Research>,
    /// The Research Waypoint whose close this is, its pane closed.
    pub(crate) closed: Option<String>,
}

/// The research thread of one Brainstorm, as the Shell holds it.
pub(crate) struct Handle {
    pub(crate) idea: String,
    pub(crate) o: Arc<Orchestrator>,
    /// Its Map is live with research in the background: new research
    /// starts.
    pub(crate) live: Arc<AtomicBool>,
    /// Parked Research Waypoints to resume; None once the thread has ended.
    resume: Arc<Mutex<Option<Vec<String>>>>,
    pub(crate) thread: JoinHandle<()>,
}

impl Handle {
    /// Resumes Parked `id`: false when the thread has ended, and the next
    /// one must take it up.
    pub(crate) fn resume(&self, id: &str) -> bool {
        match self.resume.lock().unwrap().as_mut() {
            Some(queue) => {
                queue.push(id.to_string());
                true
            }
            None => false,
        }
    }

    /// Whether `id` is one of its research sessions: its Questions are
    /// answered here.
    pub(crate) fn holds(&self, id: &str) -> bool {
        self.o.state.lock().unwrap().tickets.contains_key(id)
    }
}

/// `b`'s research on a thread of its own, its Orchestrator over `cfg`:
/// what it saved, Parked or not, taken up; new research while `live`;
/// `stop` (the Shell closing) leaves its sessions running in their panes.
pub(crate) fn spawn(
    cfg: &Config,
    b: Brainstorm,
    live: bool,
    stop: Arc<AtomicBool>,
    sender: Sender<Update>,
) -> Handle {
    let cfg = Config {
        log: Arc::new(Mutex::new(log_file(&cfg.repo))),
        ..cfg.clone()
    };
    let worktree = b.worktree.clone().into();
    let o = Arc::new(Orchestrator::research(
        cfg,
        worktree,
        &b.map,
        b.research_tab.clone(),
    ));
    let live = Arc::new(AtomicBool::new(live));
    let resume = Arc::new(Mutex::new(Some(Vec::new())));
    let idea = b.idea.clone();
    let thread = {
        let (o, live, resume) = (o.clone(), live.clone(), resume.clone());
        thread::spawn(move || {
            let mut r = Runner {
                o,
                idea: b.idea.clone(),
                map: b.map.clone(),
                research: b.research.clone(),
                running: Vec::new(),
                kept: Vec::new(),
                waiting: BTreeSet::new(),
                refused: BTreeSet::new(),
                sent: (b.research_tab, b.research),
                sender,
            };
            r.run(&live, &stop, &resume);
        })
    };
    Handle {
        idea,
        o,
        live,
        resume,
        thread,
    }
}

/// One research session's thread: its Stage's outcome.
type Running = (String, JoinHandle<Result<StageResult, StageError>>);

struct Runner {
    o: Arc<Orchestrator>,
    idea: String,
    map: String,
    /// Every Research Waypoint of the Map it holds: running, kept or Parked.
    research: Vec<Research>,
    running: Vec<Running>,
    /// Its result written with its Waypoint still open in bd: its pane and
    /// its slot kept until bd shows it closed.
    kept: Vec<String>,
    /// Said to wait for a slot, until it starts.
    waiting: BTreeSet<String>,
    /// Its claim failed, said once.
    refused: BTreeSet<String>,
    /// The tab and the research last sent to the Shell.
    sent: (String, Vec<Research>),
    sender: Sender<Update>,
}

impl Runner {
    /// Each tick until the Shell closes, or its Map is no longer live and
    /// nothing of its runs, waits on bd or is to be resumed.
    fn run(&mut self, live: &AtomicBool, stop: &AtomicBool, resume: &Mutex<Option<Vec<String>>>) {
        let saved: Vec<String> = self
            .research
            .iter()
            .filter(|r| !r.parked)
            .map(|r| r.waypoint.clone())
            .collect();
        for id in saved {
            self.take(&id);
        }
        loop {
            if stop.load(Ordering::SeqCst) {
                self.o.stop();
                for (_, t) in self.running.drain(..) {
                    let _ = t.join();
                }
                return self.sync(None);
            }
            self.reap();
            for id in self.kept.clone() {
                if self.closed(&id) {
                    self.close(&id);
                }
            }
            let resumed = std::mem::take(resume.lock().unwrap().as_mut().unwrap());
            for id in resumed {
                if let Some(r) = self.research.iter_mut().find(|r| r.waypoint == id) {
                    r.parked = false;
                    self.take(&id);
                }
            }
            if live.load(Ordering::SeqCst) {
                self.start_ready();
            }
            self.sync(None);
            {
                let mut queue = resume.lock().unwrap();
                let idle = self.running.is_empty() && self.kept.is_empty();
                if !live.load(Ordering::SeqCst) && idle && queue.as_ref().unwrap().is_empty() {
                    *queue = None;
                    return;
                }
            }
            thread::sleep(self.o.cfg.tick);
        }
    }

    /// The unclaimed Research Waypoints on the frontier, in map order, each
    /// claimed and started while a slot is free, the rest said to wait.
    fn start_ready(&mut self) {
        let argv = ["ready", "--parent", &self.map, "--unassigned", "--json"];
        let Ok(ready) = self.o.bd_all(&argv) else {
            return; // bd asked again next tick
        };
        let max = app::count(&self.o.cfg.repo, &MAX_RESEARCH);
        for w in ready.iter().filter(|w| labelled(w, LABEL)) {
            let id = w.id.as_str();
            if self.research.iter().any(|r| r.waypoint == id) {
                continue;
            }
            let busy = self.running.len() + self.kept.len();
            if busy >= max {
                if self.waiting.insert(id.to_string()) {
                    let text = format!("waits for a research slot, {busy} of {max} running");
                    self.o.report(id, &text);
                }
                continue;
            }
            let claim = ["bd", "update", id, "--claim"];
            if let Err(err) = self.o.cfg.tools.run(&self.o.cfg.repo, &claim) {
                if self.refused.insert(id.to_string()) {
                    let text = format!("research not started: its claim failed: {err}");
                    self.o.report(id, &text);
                }
                continue;
            }
            self.waiting.remove(id);
            self.research.push(Research {
                waypoint: id.to_string(),
                ..Default::default()
            });
            self.take(id);
        }
    }

    /// Research Waypoint `id`'s session on a thread of its own: its saved
    /// pane watched again, else its saved session resumed by id, else
    /// brainstorm-research started fresh.
    fn take(&mut self, id: &str) {
        let Some(r) = self.research.iter().find(|r| r.waypoint == id).cloned() else {
            return;
        };
        if !r.pane.is_empty() || !r.session.id.is_empty() {
            self.o.update(id, |ts| {
                ts.stage = RESEARCH.name.to_string();
                ts.round = 0;
                if !r.pane.is_empty() {
                    ts.panes.insert(RESEARCH.name.to_string(), r.pane.clone());
                }
                ts.sessions
                    .insert(RESEARCH.name.to_string(), r.session.clone());
            });
        }
        let (o, map, id) = (self.o.clone(), self.map.clone(), id.to_string());
        let thread = {
            let id = id.clone();
            thread::spawn(move || {
                let file = o.run_dir(&id).join(result_name(&RESEARCH, 0));
                let file = file.display().to_string();
                let inputs = [
                    ("MAP", map.as_str()),
                    ("WAYPOINT", id.as_str()),
                    ("RESULT FILE", file.as_str()),
                ];
                o.run_stage(&id, &RESEARCH, 0, &inputs, ResultRequirements::default())
            })
        };
        self.running.push((id, thread));
    }

    /// Each session that ended: its Waypoint closed in bd closes its pane
    /// and frees its slot; still open, its pane and slot are kept; Parked,
    /// its pane closes, its session saved, and its slot frees.
    fn reap(&mut self) {
        let (ended, running) = std::mem::take(&mut self.running)
            .into_iter()
            .partition(|(_, t)| t.is_finished());
        self.running = running;
        for (id, t) in ended {
            match t.join() {
                Ok(Ok(_)) if self.closed(&id) => self.close(&id),
                Ok(Ok(_)) => {
                    let pane = self.pane(&id);
                    let at = self.o.locate(&pane);
                    let at = at.trim_start_matches('(').trim_end_matches(')');
                    let text = format!(
                        "result written, but {} is still open in bd: {at} kept",
                        suffix(&id)
                    );
                    self.o.report(&id, &text);
                    self.kept.push(id);
                }
                Ok(Err(StageError::Parked(reason))) => self.park(&id, &reason),
                Ok(Err(StageError::Stopped)) => {}
                Err(_) => self.o.report(&id, "research stopped: its thread died"),
            }
        }
    }

    /// Whether bd shows Research Waypoint `id` closed; a failed read is not.
    fn closed(&self, id: &str) -> bool {
        let shown = self.o.bd_all(&["show", id, "--json"]);
        shown.is_ok_and(|issues| issues.iter().any(|i| i.id == id && i.status == "closed"))
    }

    /// Its pane, as its Stage last placed it.
    fn pane(&self, id: &str) -> String {
        let ts = self.o.ticket(id);
        ts.panes.get(RESEARCH.name).cloned().unwrap_or_default()
    }

    /// Its pane closed and forgotten, already gone or not; why not said.
    fn close_pane(&self, id: &str) {
        let pane = self.pane(id);
        if pane.is_empty() {
            return;
        }
        if let Err(err) = self.o.herdr(&["pane", "close", &pane]) {
            if !herdr::pane_gone(&err) {
                self.o
                    .report(id, &format!("research pane not closed: {err}"));
            }
        }
    }

    /// `id` closed in bd: its pane closes and its slot frees.
    fn close(&mut self, id: &str) {
        self.close_pane(id);
        self.forget(id);
        self.research.retain(|r| r.waypoint != id);
        // the Shell has it saved by the time it reads the line
        self.sync(Some(id.to_string()));
        self.o.report(id, "closed; its pane closes");
    }

    /// Parked for `reason`: its pane closes, its session saved, its slot
    /// frees; /continue @<waypoint> resumes it.
    fn park(&mut self, id: &str, reason: &str) {
        let session = self.o.ticket(id).sessions.get(RESEARCH.name).cloned();
        self.close_pane(id);
        self.forget(id);
        if let Some(r) = self.research.iter_mut().find(|r| r.waypoint == id) {
            r.pane.clear();
            r.session = session.unwrap_or_default();
            r.parked = true;
        }
        self.sync(None);
        let text = match reason {
            AWAY => "parked: its session asked while you were Away".to_string(),
            reason => format!("parked: {reason}"),
        };
        self.o.report(id, &text);
    }

    /// Its Stage's record dropped, and its place in kept.
    fn forget(&mut self, id: &str) {
        self.o.state.lock().unwrap().tickets.remove(id);
        self.kept.retain(|k| k != id);
    }

    /// The panes and sessions its Stages hold copied in, sent to the Shell
    /// when they changed or `closed` names a close.
    fn sync(&mut self, closed: Option<String>) {
        for r in self.research.iter_mut().filter(|r| !r.parked) {
            let ts = self.o.ticket(&r.waypoint);
            if let Some(pane) = ts.panes.get(RESEARCH.name) {
                r.pane = pane.clone();
            }
            if let Some(session) = ts.sessions.get(RESEARCH.name) {
                r.session = session.clone();
            }
        }
        let tab = self.o.place.as_ref().unwrap().tab.lock().unwrap().clone();
        if closed.is_none() && self.sent == (tab.clone(), self.research.clone()) {
            return;
        }
        self.sent = (tab.clone(), self.research.clone());
        let _ = self.sender.send(Update {
            idea: self.idea.clone(),
            tab,
            research: self.research.clone(),
            closed,
        });
    }
}
