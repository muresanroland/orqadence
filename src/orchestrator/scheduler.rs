//! The scheduler: starts ready Tickets, at most max_tickets at once, each on
//! a thread of its own that is never joined (ADR 0003), resumes the ones a
//! stopped run left behind, polls PRs for merges (ADR 0002) and obeys the
//! Shell's commands. Its Tickets are an Epic's children, or a Ticket run's
//! queue, which the Shell adds to while it runs.

use serde::Deserialize;
use std::fs;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use super::app;
use super::result::ResultRequirements;
use super::stage::{pr_ref, result_name, Orchestrator, StageError, ADDRESS};
use super::state::{TicketState, STATUS_MERGED, STATUS_PARKED, STATUS_PR_OPEN, STATUS_RUNNING};

/// One row of a bd JSON reply, the fields the scheduler and the Shell read.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct BdIssue {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) status: String,
    pub(crate) issue_type: String,
    pub(crate) parent: String,
    pub(crate) dependencies: Vec<BdDependency>,
    /// 'PR merged: <url>' on a Ticket poll_merges closed.
    pub(crate) close_reason: String,
    /// Its own bd labels, never its Epic's.
    pub(crate) labels: Vec<String>,
}

impl BdIssue {
    /// The Tickets that must close before this one: its bd blocks dependencies.
    pub(crate) fn blockers(&self) -> impl Iterator<Item = &str> {
        self.dependencies
            .iter()
            .filter(|d| d.kind == "blocks")
            .map(|d| d.depends_on_id.as_str())
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct BdDependency {
    pub(crate) depends_on_id: String,
    #[serde(rename = "type")]
    pub(crate) kind: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct GhPr {
    state: String,
    mergeable: String,
}

impl Orchestrator {
    fn bd_issues(&self, args: &[&str]) -> Result<Vec<BdIssue>, String> {
        let mut argv = vec!["bd"];
        argv.extend_from_slice(args);
        let out = self
            .cfg
            .tools
            .run(&self.cfg.repo, &argv)
            .map_err(|err| err.to_string())?;
        let issues: Option<Vec<BdIssue>> = serde_json::from_str(&out)
            .map_err(|err| format!("bd {}: unreadable reply: {err}", args[0]))?;
        Ok(issues
            .unwrap_or_default()
            .into_iter()
            .filter(|issue| issue.issue_type != "epic")
            .collect())
    }

    /// The Ticket's Ticket labels, by name: its own orqa: labels as bd
    /// shows them now, read as each Stage starts, like the rows. Every other
    /// bd label is ignored.
    pub(super) fn labels(&self, ticket: &str) -> Result<Vec<String>, String> {
        let issues = self.bd_issues(&["show", ticket, "--json"])?;
        Ok(issues
            .iter()
            .flat_map(|issue| &issue.labels)
            .filter_map(|label| label.strip_prefix("orqa:"))
            .map(String::from)
            .collect())
    }

    /// The run's Tickets as bd filters them: the Epic's children, or with
    /// no Epic the Ticket run's queue by id; None for a queue with none left.
    fn scope(&self, epic: &str) -> Option<[String; 2]> {
        if !epic.is_empty() {
            return Some(["--parent".to_string(), epic.to_string()]);
        }
        let queue = self.state.lock().unwrap().queue.join(",");
        (!queue.is_empty()).then(|| ["--id".to_string(), queue])
    }

    /// The run's Tickets, every status; None says "bd list failed".
    fn children(&self, epic: &str) -> Option<Vec<BdIssue>> {
        let Some([by, value]) = self.scope(epic) else {
            return Some(Vec::new());
        };
        match self.bd_issues(&["list", &by, &value, "--all", "--limit", "0", "--json"]) {
            Ok(children) => Some(children),
            Err(err) => {
                self.report("", &format!("bd list failed: {err}"));
                None
            }
        }
    }

    /// The run's Tickets bd has ready: open, every blocker closed; a Ticket
    /// run's in the order they were added. bd's ready query takes no --id,
    /// so a Ticket run's is every ready Ticket, kept if queued.
    fn ready(&self, epic: &str) -> Result<Vec<BdIssue>, String> {
        if !epic.is_empty() {
            return self.bd_issues(&["ready", "--parent", epic, "--json"]);
        }
        let queue = self.state.lock().unwrap().queue.clone();
        if queue.is_empty() {
            return Ok(Vec::new());
        }
        let mut ready = self.bd_issues(&["ready", "--limit", "0", "--json"])?;
        ready.retain(|i| queue.contains(&i.id));
        ready.sort_by_key(|i| queue.iter().position(|id| *id == i.id));
        Ok(ready)
    }

    /// Whether the run may end on the Tickets bd listed for `queued`: not
    /// if a Ticket joined the queue since. Once it says yes enqueue refuses;
    /// both under the state lock, so no Ticket joins a finished run.
    fn finish(&self, queued: &[String]) -> bool {
        let state = self.state.lock().unwrap();
        let done = state.queue == queued;
        self.done.store(done, Ordering::SeqCst);
        done
    }

    /// Drives an Epic, or with none the Ticket run over the State's queue: it
    /// starts ready Tickets, at most max_tickets at once, resumes the ones a
    /// stopped run left behind, polls PRs for merges, obeys the Shell's
    /// commands, and returns when every one of its Tickets is closed (a
    /// Ticket run too when none is left in it) or on /stop-work.
    pub(crate) fn run(self: &Arc<Self>, epic: &str) -> Result<(), String> {
        self.change_state(|state| {
            state.epic = epic.to_string();
            if !epic.is_empty() {
                // bd has a started Ticket /remove-ticket took out in progress:
                // it goes on where it left off, or the Epic never ends
                state.tickets.append(&mut state.removed);
                return state.queue.clear();
            }
            // a Ticket run saved before the queue: its Tickets not merged join first
            let saved: Vec<String> = state
                .tickets
                .iter()
                .filter(|(id, ts)| ts.status != STATUS_MERGED && !state.queue.contains(id))
                .map(|(id, _)| id.clone())
                .collect();
            state.queue.splice(0..0, saved);
        });

        let launch = |ticket: &str, work: fn(&Orchestrator, &str)| {
            // A Ticket can consume stop while the scheduler is in a bd call.
            // Its saved state stays running for resume, but this run is over.
            if self.stopping() {
                return;
            }
            self.active.lock().unwrap().insert(ticket.to_string());
            let slot = Slot {
                o: Arc::clone(self),
                ticket: ticket.to_string(),
            };
            let handle = thread::spawn(move || work(&slot.o, &slot.ticket));
            #[cfg(test)]
            self.threads.lock().unwrap().push(handle);
            #[cfg(not(test))]
            let _ = handle; // never joined: /stop-work must not wait out a Stage
        };
        let busy = |ticket: &str| self.active.lock().unwrap().contains(ticket);
        let in_pipeline = || self.active.lock().unwrap().len();

        let mut last_poll: Option<Instant> = None;
        loop {
            for command in self.commands() {
                let (kind, ticket) = command.split_once('-').unwrap_or((&command, ""));
                let ts = self.ticket(ticket);
                if kind == "remove" && self.consume(&command) {
                    self.remove(ticket, busy(ticket));
                } else if busy(ticket) || ticket.is_empty() {
                    // a running Ticket's own waits consume its retry and
                    // park, and sleep owns stop, in every mode; a Question's
                    // answers, nudge among them, go by Orchestrator::answer
                } else if kind == "retry" && ts.status == STATUS_PARKED && self.consume(&command) {
                    // a fresh session: the Parked one is closed, not watched or resumed
                    if let Some(old) = ts.panes.get(&ts.stage) {
                        let _ = self.herdr(&["pane", "close", old]);
                    }
                    self.update(ticket, |ts| {
                        ts.status = STATUS_RUNNING.to_string();
                        let stage = ts.stage.clone();
                        ts.panes.remove(&stage);
                        ts.sessions.remove(&stage);
                    });
                } else if kind == "continue" && ts.status == STATUS_PARKED && self.consume(&command)
                {
                    // /continue @ticket: its live session watched again, its question asked
                    self.update(ticket, |ts| ts.status = STATUS_RUNNING.to_string());
                } else if kind == "address" && self.consume(&command) {
                    launch(ticket, Orchestrator::address);
                } else if ts.status.is_empty() && self.consume(&command) {
                    self.report(ticket, "refused: not a Ticket of this run");
                } else if self.consume(&command) {
                    self.report(ticket, "ignored: not waiting on a Wake");
                }
            }
            if last_poll.is_none_or(|at| at.elapsed() >= self.cfg.poll_prs) {
                self.poll_merges();
                last_poll = Some(Instant::now());
            }

            let queued = self.state.lock().unwrap().queue.clone();
            match self.children(epic) {
                None => {}
                Some(children) if children.is_empty() && !epic.is_empty() => {
                    return Err(format!(
                        "{epic} has no Tickets: is it the id of a beads Epic in this repo?"
                    ))
                }
                Some(children)
                    if children.iter().all(|c| c.status == "closed") && self.finish(&queued) =>
                {
                    let text = match (epic.is_empty(), children.is_empty()) {
                        (true, true) => "Ticket run done, no Ticket left in it",
                        (true, false) => "Ticket run done, every Ticket closed",
                        (false, _) => "Epic done, every Ticket closed",
                    };
                    self.report("", text);
                    return Ok(());
                }
                Some(_) => {}
            }

            let max = app::max_tickets(&self.cfg.repo);
            for ticket in self.resumable() {
                if !busy(&ticket) && in_pipeline() < max {
                    launch(&ticket, Orchestrator::run_ticket);
                }
            }
            match self.ready(epic) {
                Err(err) => self.report("", &format!("bd ready failed: {err}")),
                Ok(ready) => {
                    for issue in ready {
                        if self.ticket(&issue.id).status.is_empty() && in_pipeline() < max {
                            self.update(&issue.id, |_| {});
                            launch(&issue.id, Orchestrator::run_ticket);
                        }
                    }
                }
            }
            if !self.sleep() {
                return Ok(()); // /stop-work is a clean end, not a failure
            }
        }
    }

    /// Adds Tickets to the Ticket run's queue, each once, a removed one with
    /// the state it left with; the scheduler starts them as slots free, the
    /// order kept. False, nothing added, once the run is finishing.
    pub(crate) fn enqueue(&self, tickets: &[String]) -> bool {
        let mut added = false;
        self.change_state(|state| {
            if self.done.load(Ordering::SeqCst) {
                return;
            }
            added = true;
            for ticket in tickets {
                if !state.queue.contains(ticket) {
                    state.queue.push(ticket.clone());
                }
                if let Some(ts) = state.removed.remove(ticket) {
                    state.tickets.insert(ticket.clone(), ts);
                }
            }
        });
        added
    }

    /// /remove-ticket, on the scheduler's thread, the one that starts
    /// Tickets: a Ticket with nothing running leaves the Ticket run (a
    /// queued one, a Parked one, one whose PR is open, which stays open
    /// unpolled), its state kept aside for a /start-ticket that adds it
    /// back; a working one is refused.
    fn remove(&self, ticket: &str, working: bool) {
        if working {
            return self.report(ticket, "remove refused: working, /park it first");
        }
        let pr = self.ticket(ticket).status == STATUS_PR_OPEN;
        self.change_state(|state| {
            state.queue.retain(|t| t != ticket);
            if let Some(ts) = state.tickets.remove(ticket) {
                state.removed.insert(ticket.to_string(), ts);
            }
        });
        self.report(
            ticket,
            match pr {
                true => "removed from the run, its PR stays open",
                false => "removed from the run",
            },
        );
    }

    /// Tells each open Ticket of the run that depends on `ticket` that it now
    /// waits on the PR's merge (ADR 0002). With no run there is nothing
    /// waiting.
    pub(crate) fn wait_dependents(&self, ticket: &str, pr: &str) {
        let epic = self.state.lock().unwrap().epic.clone();
        let Some(children) = self.children(&epic) else {
            return;
        };
        let suffix = ticket.rsplit('.').next().unwrap_or(ticket);
        for child in children {
            let blocked = child.blockers().any(|id| id == ticket);
            if blocked && child.status != "closed" {
                self.report(
                    &child.id,
                    &format!("waiting for {} to merge (Ticket {suffix})", pr_ref(pr)),
                );
            }
        }
    }

    /// The Tickets the state file says are in the Pipeline.
    pub(crate) fn resumable(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .tickets
            .iter()
            .filter(|(_, ts)| ts.status == STATUS_RUNNING)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Asks gh about every open PR. A merge is what closes a Ticket and so
    /// unblocks its dependents (ADR 0002).
    fn poll_merges(&self) {
        let open: Vec<(String, TicketState)> = self
            .state
            .lock()
            .unwrap()
            .tickets
            .iter()
            .filter(|(_, ts)| ts.status == STATUS_PR_OPEN)
            .map(|(id, ts)| (id.clone(), ts.clone()))
            .collect();
        let (tools, repo) = (&self.cfg.tools, &self.cfg.repo);

        for (ticket, ts) in open {
            let view = tools
                .run(
                    repo,
                    &["gh", "pr", "view", &ts.pr, "--json", "state,mergeable"],
                )
                .map_err(|err| err.to_string())
                .and_then(|out| serde_json::from_str::<GhPr>(&out).map_err(|err| err.to_string()));
            let pr = match view {
                Ok(pr) => pr,
                Err(err) => {
                    self.log(&ticket, &format!("gh pr view failed: {err}"));
                    continue;
                }
            };
            if pr.state == "MERGED" {
                // Closing the Ticket is what unblocks its dependents: until bd
                // has done it the Ticket stays pr-open and the next poll tries
                // again.
                let reason = format!("PR merged: {}", ts.pr);
                if let Err(err) = tools.run(repo, &["bd", "close", &ticket, "--reason", &reason]) {
                    self.log(
                        &ticket,
                        &format!("merged but not closed, will retry: {err}"),
                    );
                    continue;
                }
                // The work is on main now, so bd's cleanliness and containment
                // checks (which a squash merge fails) no longer protect anything.
                let worktree = self.worktree(&ticket).display().to_string();
                if let Err(err) =
                    tools.run(repo, &["bd", "worktree", "remove", &worktree, "--force"])
                {
                    self.log(
                        &ticket,
                        &format!("could not remove the worktree, remove it by hand: {err}"),
                    );
                } else if let Err(err) = tools.run(repo, &["git", "branch", "-D", &ticket]) {
                    self.log(
                        &ticket,
                        &format!("worktree removed, could not delete the branch: {err}"),
                    );
                }
                self.update(&ticket, |ts| ts.status = STATUS_MERGED.to_string());
                self.report(&ticket, "merged, Ticket closed");
            } else if pr.state == "CLOSED" {
                self.update(&ticket, |ts| {
                    ts.status = STATUS_PARKED.to_string();
                    ts.reason = "PR closed without merging".to_string();
                });
                self.report(
                    &ticket,
                    &format!("parked: {} closed without merging", pr_ref(&ts.pr)),
                );
            } else if pr.mergeable == "CONFLICTING" && !ts.conflict {
                self.update(&ticket, |ts| ts.conflict = true);
                self.report(
                    &ticket,
                    &format!(
                        "{} conflicts with main, /address resolves it",
                        pr_ref(&ts.pr)
                    ),
                );
            } else if pr.mergeable == "MERGEABLE" && ts.conflict {
                self.update(&ticket, |ts| ts.conflict = false);
            }
        }
    }

    /// Runs the address Stage for a Ticket with an open PR, on the user's
    /// command only: a fresh session in the kept worktree, fed the PR's
    /// review comments and whether it conflicts with main.
    fn address(&self, ticket: &str) {
        let ts = self.ticket(ticket);
        if ts.status != STATUS_PR_OPEN {
            self.report(ticket, "address refused: no open PR");
            return;
        }
        let feedback = match self.cfg.tools.run(
            &self.cfg.repo,
            &[
                "gh",
                "pr",
                "view",
                &ts.pr,
                "--json",
                "mergeable,reviews,comments",
            ],
        ) {
            Ok(feedback) => feedback,
            Err(err) => {
                self.report(ticket, &format!("address failed: {err}"));
                return;
            }
        };
        // every address run is a new one, never an earlier one resumed
        let _ = fs::remove_file(self.run_dir(ticket).join(result_name(&ADDRESS, 0)));
        self.update(ticket, |ts| {
            ts.sessions.remove(ADDRESS.name);
        });
        let inputs = address_inputs(&ts.pr, &feedback);
        let inputs: Vec<(&str, &str)> = inputs
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let result = self.run_stage(ticket, &ADDRESS, 0, &inputs, ResultRequirements::default());
        if self.stopping() {
            return;
        }
        match result {
            Ok(_) => {
                let tab = self.ticket(ticket).tab;
                if !tab.is_empty() {
                    let _ = self.herdr(&["tab", "close", &tab]);
                    self.update(ticket, |ts| {
                        ts.tab.clear();
                        ts.panes.clear();
                        ts.sessions.clear();
                    });
                }
                self.report(ticket, &format!("addressed {}", pr_ref(&ts.pr)));
            }
            Err(StageError::Parked(reason)) => {
                self.report(ticket, &format!("address gave up: {reason}"));
            }
            Err(StageError::Stopped) => self.close_on_limit(ticket),
        }
    }
}

/// A Ticket's place in the Pipeline, given back when its thread ends, also
/// by a panic.
struct Slot {
    o: Arc<Orchestrator>,
    ticket: String,
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.o.active.lock().unwrap().remove(&self.ticket);
    }
}

/// The address Stage's inputs, from gh's view of the PR.
pub(crate) fn address_inputs(pr: &str, gh_json: &str) -> Vec<(String, String)> {
    let mut conflicts = "unknown, check with gh";
    if let Ok(view) = serde_json::from_str::<GhPr>(gh_json) {
        if !view.mergeable.is_empty() && view.mergeable != "UNKNOWN" {
            conflicts = if view.mergeable == "CONFLICTING" {
                "yes"
            } else {
                "no"
            };
        }
    }
    [
        ("PR", pr),
        ("Conflicts with main", conflicts),
        ("Review comments (gh JSON)", gh_json.trim()),
    ]
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .to_vec()
}
