//! The Release: the Stage a run carrying orqa:release ends in, once every
//! Ticket of the run is merged. It belongs to the run, not to a Ticket: its
//! record is the State's release, apart from the Tickets', and its id
//! (release-<epic>, release-<epic>-<date>-<time> when an earlier Release's
//! worktree has that name, or release-<date>-<time> in a Ticket run, of
//! which a day may have several) names its worktree, Run directory, branch
//! and tab as a Ticket's id names its own.

use std::fs;

use super::app::{self, RELEASE_ON};
use super::result::ResultRequirements;
use super::scheduler::BdIssue;
use super::stage::{pr_ref, result_name, Orchestrator, StageError, RELEASE};
use super::state::{Release, STATUS_MERGED, STATUS_RUNNING};

/// The Release label: on an Epic, or on any Ticket of a Ticket run, it asks
/// that the run end in a Release.
pub(crate) const RELEASE_LABEL: &str = "orqa:release";

impl Orchestrator {
    /// Whether the run, every Ticket closed, ends in a Release, read now so
    /// a label added mid-run counts: the Epic Input ("<epic> <title>", or
    /// "none" in a Ticket run) when it does. A Release already started goes
    /// on as it started. Otherwise none with release_on off, or when no
    /// Ticket of the run merged a PR; an Epic run reads the Epic's own
    /// labels, never the copies bd put on its Tickets, a Ticket run any of
    /// its Tickets'. Err when bd cannot show the Epic.
    pub(super) fn release_due(
        &self,
        epic: &str,
        children: &[BdIssue],
    ) -> Result<Option<String>, String> {
        let saved = self.state.lock().unwrap().release.is_some();
        let merged = || {
            children
                .iter()
                .any(|c| self.ticket(&c.id).status == STATUS_MERGED)
        };
        if !saved && !(app::switch(&self.cfg.repo, &RELEASE_ON) && merged()) {
            return Ok(None);
        }
        let carries = |issue: &BdIssue| issue.labels.iter().any(|l| l == RELEASE_LABEL);
        if epic.is_empty() {
            let due = saved || children.iter().any(carries);
            return Ok(due.then(|| "none".to_string()));
        }
        let shown = self.bd_all(&["show", epic, "--json"])?;
        let shown = shown.into_iter().next().unwrap_or_default();
        Ok((saved || carries(&shown)).then(|| format!("{epic} {}", shown.title)))
    }

    /// Runs the run's Release on the scheduler's thread: its record made
    /// once, its worktree off origin's default branch, then its Stage
    /// through run_stage, fed the Bump (minor for an Epic, patch for a
    /// Ticket run), the Epic and each merged Ticket with its PR. Done, its
    /// tab closes and its version and PR go into its record. A long usage
    /// limit leaves its session saved; a park stops the run as /stop-work
    /// does, the Release saved for /continue.
    pub(super) fn release(&self, epic: &str, epic_input: &str, children: &[BdIssue]) {
        let fresh = self.state.lock().unwrap().release.is_none();
        let stamp = (self.cfg.clock)().format("%Y-%m-%d-%H%M%S");
        let (mut id, bump) = if epic.is_empty() {
            (format!("release-{stamp}"), "patch")
        } else if fresh && self.worktree(&format!("release-{epic}")).exists() {
            // an earlier Release's worktree, its branch off an older main
            (format!("release-{epic}-{stamp}"), "minor")
        } else {
            (format!("release-{epic}"), "minor")
        };
        self.change_state(|state| {
            let release = state.release.get_or_insert_with(|| {
                Box::new(Release {
                    id: id.clone(),
                    ..Default::default()
                })
            });
            release.ts.status = STATUS_RUNNING.to_string();
            release.ts.reason.clear();
            id = release.id.clone(); // a saved one's, from its start
        });
        if fresh {
            // an earlier run's Release of the same id is not this one's
            let _ = fs::remove_file(self.run_dir(&id).join(result_name(&RELEASE, 0)));
        }
        let merged: Vec<String> = children
            .iter()
            .filter_map(|c| {
                let url = c.close_reason.strip_prefix("PR merged: ")?;
                Some(format!("{} {}: {url}", c.id, c.title))
            })
            .collect();
        let tickets = format!("\n  - {}", merged.join("\n  - "));
        let inputs = [("Bump", bump), ("Epic", epic_input), ("Tickets", &tickets)];
        let want = ResultRequirements::default();
        let ended = self
            .prepare_worktree(&id)
            .and_then(|()| self.run_stage(&id, &RELEASE, 0, &inputs, want));
        match ended {
            Ok(result) => {
                let tab = self.ticket(&id).tab;
                if !tab.is_empty() {
                    let _ = self.herdr(&["tab", "close", &tab]);
                }
                self.change_state(|state| {
                    if let Some(release) = &mut state.release {
                        release.version = result.version.clone();
                        release.ts.pr = result.pr.clone();
                        release.ts.tab.clear();
                        release.ts.panes.clear();
                        release.ts.sessions.clear();
                    }
                });
                self.report("", &format!("release done: {}", result.version));
                if !result.pr.is_empty() {
                    let text = format!("version {} opened", pr_ref(&result.pr));
                    self.emit("", &text, true, &result.pr);
                }
            }
            Err(StageError::Stopped) => self.close_on_limit(&id),
            Err(StageError::Parked(reason)) => {
                self.park(&id, &reason);
                self.stop();
            }
        }
    }
}
