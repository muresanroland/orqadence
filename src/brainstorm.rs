//! The Brainstorm's labels and its state: one folder per Brainstorm,
//! .orqadence-local/brainstorms/<idea>/, holding its state.json and its
//! result files. Never in the run's state.json, which a run clears.

use crate::orchestrator::scheduler::BdIssue;
use crate::orchestrator::stage::plural;
use crate::orchestrator::state::{is_zero, local_dir, Session, LOCAL};
use crate::shell::suffix;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The Idea, a task.
pub(crate) const IDEA: &str = "brainstorm:idea";
/// The Map, an epic. Its children inherit the label unless created with
/// --no-inherit-labels, so a Map is also an epic.
pub(crate) const MAP: &str = "brainstorm:map";
// The Waypoint kinds, on the Map's children; nothing reads them yet.
#[allow(dead_code)]
pub(crate) const GRILLING: &str = "brainstorm:grilling";
#[allow(dead_code)]
pub(crate) const PROTOTYPE: &str = "brainstorm:prototype";
#[allow(dead_code)]
pub(crate) const TASK: &str = "brainstorm:task";
/// A Research Waypoint.
pub(crate) const RESEARCH: &str = "brainstorm:research";
/// The one build-Epic Waypoint.
pub(crate) const EPIC: &str = "brainstorm:epic";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Phase {
    #[default]
    Charting,
    Map,
    Done,
}

/// One research session of the Map's, in the research-<map> tab.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Research {
    pub(crate) waypoint: String,
    pub(crate) pane: String,
    pub(crate) session: Session,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) parked: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) limited: bool,
    /// Its Stage's spent retry, nudge and waits, as a Ticket's are saved.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) retried: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) nudged: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub(crate) waits: usize,
}

/// What Orqadence knows about one Brainstorm.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Brainstorm {
    pub(crate) phase: Phase,
    pub(crate) idea: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) map: String,
    /// The live pane, and the session it runs.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) pane: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) session: Option<Session>,
    /// A Waypoint session's result file, kept for its resume.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) result: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) worktree: String,
    /// brainstorm/<idea>.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) branch: String,
    /// The Tickets charting came out as.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) tickets: Vec<String>,
    /// The start-Map modal's answer: the research runs in the background.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) background: bool,
    /// Start Map or Continue pressed once: a Map not started never was.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) started: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) research_tab: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) research: Vec<Research>,
    /// The LABEL lines not yet answered.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) label_lines: Vec<String>,
    /// The Epics written, in build order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) epics: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) docs_pr: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) docs_merged: bool,
}

fn brainstorms(repo: &Path) -> PathBuf {
    repo.join(LOCAL).join("brainstorms")
}

impl Brainstorm {
    /// Writes its state.json atomically: a reader sees the old or the new
    /// state, never half of one. Each write has a tmp file of its own, as a
    /// driver's thread and the Shell can save at once.
    pub(crate) fn save(&self, repo: &Path) -> io::Result<()> {
        static WRITES: AtomicUsize = AtomicUsize::new(0);
        local_dir(repo)?;
        let dir = brainstorms(repo).join(&self.idea);
        fs::create_dir_all(&dir)?;
        let raw = serde_json::to_vec_pretty(self)?;
        let n = WRITES.fetch_add(1, Ordering::SeqCst);
        let tmp = dir.join(format!("state.json.{n}.tmp"));
        fs::write(&tmp, raw)?;
        fs::rename(&tmp, dir.join("state.json")).inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })
    }

    /// Done with a docs PR not yet merged.
    pub(crate) fn docs_waiting(&self) -> bool {
        self.phase == Phase::Done && !self.docs_pr.is_empty() && !self.docs_merged
    }

    /// A done one's close in bd, and its reason: the Map its Epics close,
    /// or the Idea its Tickets close.
    pub(crate) fn closing(&self) -> Option<(String, String)> {
        match self.phase {
            Phase::Done if !self.epics.is_empty() => {
                Some((self.map.clone(), format!("Epics {}", self.epics.join(", "))))
            }
            Phase::Done if !self.tickets.is_empty() => Some((
                self.idea.clone(),
                format!("Tickets {}", self.tickets.join(", ")),
            )),
            _ => None,
        }
    }

    /// What /continue @ takes: the Map once there is one, else the Idea.
    pub(crate) fn key(&self) -> &str {
        if self.map.is_empty() {
            &self.idea
        } else {
            &self.map
        }
    }
}

/// Whether `i` carries bd label `label`.
pub(crate) fn labelled(i: &BdIssue, label: &str) -> bool {
    i.labels.iter().any(|l| l == label)
}

/// The saved Brainstorm whose state.json changed last, a done one never.
pub(crate) fn most_recent<'a>(repo: &Path, saved: &'a [Brainstorm]) -> Option<&'a Brainstorm> {
    let changed = |b: &Brainstorm| {
        let file = brainstorms(repo).join(&b.idea).join("state.json");
        fs::metadata(file).and_then(|m| m.modified()).ok()
    };
    saved
        .iter()
        .filter(|b| b.phase != Phase::Done)
        .max_by_key(|b| changed(b))
}

/// Every saved Brainstorm, by idea, but one bd shows thrown away: its Idea
/// (while charting) or its Map closed by the user. A done one's Map
/// Orqadence closed itself, so it stays. An issue bd does not list keeps it,
/// and an unreadable state.json is skipped.
pub(crate) fn load(repo: &Path, issues: &[BdIssue]) -> Vec<Brainstorm> {
    let Ok(entries) = fs::read_dir(brainstorms(repo)) else {
        return Vec::new();
    };
    let closed = |id: &str| issues.iter().any(|i| i.id == id && i.status == "closed");
    let mut saved: Vec<Brainstorm> = entries
        .flatten()
        .filter_map(|e| fs::read(e.path().join("state.json")).ok())
        .filter_map(|raw| serde_json::from_slice::<Brainstorm>(&raw).ok())
        .filter(|b| match b.phase {
            Phase::Charting => !closed(&b.idea),
            Phase::Map => !closed(&b.map),
            Phase::Done => true,
        })
        .collect();
    saved.sort_by(|a, b| a.idea.cmp(&b.idea));
    saved
}

/// What a Brainstorm's issue is: never a Ticket or an Epic of the Pipeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Kind {
    Map,
    Waypoint,
    Idea,
}

fn is_map(issue: &BdIssue) -> bool {
    issue.issue_type == "epic" && labelled(issue, MAP)
}

/// The issue's Brainstorm kind among `issues`, None for every other issue:
/// a Map, a Map's child, or an Idea.
pub(crate) fn kind(issues: &[BdIssue], issue: &BdIssue) -> Option<Kind> {
    if is_map(issue) {
        Some(Kind::Map)
    } else if !issue.parent.is_empty() && issues.iter().any(|p| p.id == issue.parent && is_map(p)) {
        Some(Kind::Waypoint)
    } else if labelled(issue, IDEA) {
        Some(Kind::Idea)
    } else {
        None
    }
}

/// The refusal for /start-epic or /start-ticket on `id`, when it is a
/// Brainstorm's issue among `issues`.
pub(crate) fn refusal(issues: &[BdIssue], id: &str) -> Option<String> {
    let issue = issues.iter().find(|i| i.id == id)?;
    Some(match kind(issues, issue)? {
        Kind::Map => format!("refused: {id} is a Map, /continue @{id} works it"),
        Kind::Waypoint => "refused: a Waypoint never enters the Pipeline".to_string(),
        Kind::Idea => "refused: an Idea never enters the Pipeline".to_string(),
    })
}

/// Why /continue @<id>, a Waypoint of `b`'s Map among `issues`, is
/// refused: closed, its research running in the background, the build-Epic
/// one with others open, or blocked; None when it can be taken.
pub(crate) fn waypoint_refusal(issues: &[BdIssue], b: &Brainstorm, id: &str) -> Option<String> {
    let w = issues.iter().find(|i| i.id == id)?;
    let open = |id: &str| issues.iter().any(|i| i.id == id && i.status != "closed");
    let name = suffix(id);
    if w.status == "closed" {
        return Some(format!("refused: {name} is closed"));
    }
    if b.research.iter().any(|r| r.waypoint == id && !r.parked) {
        return Some(format!(
            "refused: {name} is a Research Waypoint a background session is running"
        ));
    }
    let others = issues
        .iter()
        .filter(|i| i.parent == b.map && i.id != id && i.status != "closed")
        .count();
    if labelled(w, EPIC) && others > 0 {
        let are = if others == 1 { "is" } else { "are" };
        let others = plural(others, "other Waypoint");
        return Some(format!(
            "refused: {name} writes the Epic, and {others} {are} open"
        ));
    }
    let on: Vec<&str> = w.blockers().filter(|b| open(b)).map(suffix).collect();
    (!on.is_empty()).then(|| format!("refused: {name} is blocked on {}", on.join(", ")))
}

pub(crate) mod driver;
pub(crate) mod research;

#[cfg(test)]
mod brainstorm_test;
