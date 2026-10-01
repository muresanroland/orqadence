//! The scheduler: starts ready Tickets, at most max_tickets at once, each on
//! a thread of its own that is never joined (ADR 0003), resumes the ones a
//! stopped run left behind, polls PRs for merges (ADR 0002) and conflicts,
//! and obeys the Shell's commands. PR sessions, Rebase and Address PR
//! comments, count apart: at most max_pr_sessions at once, one per Ticket,
//! a PR Stage that /continue takes back included.
//! Its Tickets are an Epic's children, or a Ticket run's queue, which the
//! Shell adds to while it runs.

use serde::Deserialize;
use std::collections::BTreeSet;
use std::fs;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use super::app;
use super::pr::{self, Item};
use super::result::ResultRequirements;
use super::stage::{
    pr_ref, result_name, stage_label, Orchestrator, Stage, StageError, ADDRESS_PR_COMMENTS, AWAY,
    REBASE,
};
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
    pub(crate) description: String,
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

/// How long a PR's head holds before the poll trusts that nothing is at
/// work on it: a bot's context appears seconds after a push.
const QUIET: chrono::TimeDelta = chrono::TimeDelta::seconds(60);

/// What a Ticket's thread runs: its Pipeline or a PR session.
type Work = fn(&Orchestrator, &str);

/// The PR Stage a Ticket goes back to, never its Pipeline: Parked from it
/// (its question while Away), or running again for /continue to take it
/// back. A PR that closed unmerged clears the Stage.
fn pr_stage(ts: &TicketState) -> Option<&'static Stage> {
    let back = [STATUS_PARKED, STATUS_RUNNING].contains(&ts.status.as_str());
    [&REBASE, &ADDRESS_PR_COMMENTS]
        .into_iter()
        .find(|st| back && st.name == ts.stage)
}

impl Orchestrator {
    /// Every issue bd's reply to args holds, Epics included.
    fn bd_all(&self, args: &[&str]) -> Result<Vec<BdIssue>, String> {
        let mut argv = vec!["bd"];
        argv.extend_from_slice(args);
        let out = self
            .cfg
            .tools
            .run(&self.cfg.repo, &argv)
            .map_err(|err| err.to_string())?;
        let issues: Option<Vec<BdIssue>> = serde_json::from_str(&out)
            .map_err(|err| format!("bd {}: unreadable reply: {err}", args[0]))?;
        Ok(issues.unwrap_or_default())
    }

    /// The Tickets bd's reply to args holds: never an Epic.
    fn bd_issues(&self, args: &[&str]) -> Result<Vec<BdIssue>, String> {
        Ok(self
            .bd_all(args)?
            .into_iter()
            .filter(|issue| issue.issue_type != "epic")
            .collect())
    }

    /// The description of the Ticket's parent Epic, as bd shows it now;
    /// "" for a Ticket without a parent.
    pub(super) fn epic_description(&self, ticket: &str) -> Result<String, String> {
        let shown = self.bd_issues(&["show", ticket, "--json"])?;
        let Some(parent) = shown
            .first()
            .map(|issue| &issue.parent)
            .filter(|p| !p.is_empty())
        else {
            return Ok(String::new());
        };
        let epic = self.bd_all(&["show", parent, "--json"])?;
        Ok(epic
            .into_iter()
            .next()
            .map(|e| e.description)
            .unwrap_or_default())
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
    /// commands, queueing PR sessions oldest first past max_pr_sessions, and
    /// returns when every one of its Tickets is closed (a Ticket run too
    /// when none is left in it) or on /stop-work.
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

        let launch = |ticket: &str, pr: Option<&'static str>, work: Work| {
            // A Ticket can consume stop while the scheduler is in a bd call.
            // Its saved state stays running for resume, but this run is over.
            if self.stopping() {
                return;
            }
            self.active.lock().unwrap().insert(ticket.to_string(), pr);
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
        let busy = |ticket: &str| self.active.lock().unwrap().contains_key(ticket);
        let in_pipeline = || {
            let active = self.active.lock().unwrap();
            active.values().filter(|st| st.is_none()).count()
        };
        let pr_sessions = || {
            let active = self.active.lock().unwrap();
            active.values().filter(|st| st.is_some()).count()
        };
        // the PR sessions told they wait, each told once
        let mut waiting = BTreeSet::new();

        let mut last_poll: Option<Instant> = None;
        loop {
            // polled before the commands: after a restart /rebase goes by a
            // poll of this run, not the false `conflicting` starts with
            if last_poll.is_none_or(|at| at.elapsed() >= self.cfg.poll_prs) {
                self.poll_merges();
                last_poll = Some(Instant::now());
            }
            let max_pr = app::count(&self.cfg.repo, &app::MAX_PR_SESSIONS);
            // one view of what runs per pass: a slot freed during it goes
            // to the oldest command waiting, on the next
            let mut running = self.active.lock().unwrap().clone();
            for command in self.commands() {
                let (kind, ticket) = match command.strip_prefix("address-pr-comments-") {
                    Some(ticket) => ("address-pr-comments", ticket),
                    None => command.split_once('-').unwrap_or((&command, "")),
                };
                let ts = self.ticket(ticket);
                let pr: Option<(&Stage, Work)> = match kind {
                    "rebase" => Some((&REBASE, Orchestrator::rebase)),
                    "address-pr-comments" => {
                        Some((&ADDRESS_PR_COMMENTS, Orchestrator::address_pr_comments))
                    }
                    _ => None,
                };
                if kind == "remove" && self.consume(&command) {
                    self.remove(ticket, busy(ticket));
                } else if let Some((st, work)) = pr {
                    // PR sessions queue here, oldest first: past
                    // max_pr_sessions, and behind the Ticket's own session
                    let sessions = running.values().filter(|st| st.is_some()).count();
                    if running.contains_key(ticket) || sessions >= max_pr {
                        if waiting.insert(command.clone()) {
                            let text = format!("{} waits for a slot", stage_label(st, 0));
                            self.report(ticket, &text);
                        }
                    } else if self.consume(&command) {
                        waiting.remove(&command);
                        running.insert(ticket.to_string(), Some(st.name));
                        launch(ticket, Some(st.name), work);
                    }
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
                } else if ts.status.is_empty() && self.consume(&command) {
                    self.report(ticket, "refused: not a Ticket of this run");
                } else if self.consume(&command) {
                    self.report(ticket, "ignored: not waiting on a Wake");
                }
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

            let max = app::count(&self.cfg.repo, &app::MAX_TICKETS);
            for ticket in self.resumable() {
                if busy(&ticket) {
                    continue;
                }
                // one going back to its PR Stage takes a PR-session slot
                match pr_stage(&self.ticket(&ticket)) {
                    Some(st) if pr_sessions() < max_pr => {
                        launch(&ticket, Some(st.name), Orchestrator::continue_pr)
                    }
                    None if in_pipeline() < max => launch(&ticket, None, Orchestrator::run_ticket),
                    _ => {}
                }
            }
            match self.ready(epic) {
                Err(err) => self.report("", &format!("bd ready failed: {err}")),
                Ok(ready) => {
                    for issue in ready {
                        if self.ticket(&issue.id).status.is_empty() && in_pipeline() < max {
                            self.update(&issue.id, |_| {});
                            launch(&issue.id, None, Orchestrator::run_ticket);
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

    /// The Tickets the state file says are in the Pipeline, or going back
    /// to their PR Stage.
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

    /// Asks gh about every open PR, one GraphQL query each, serially. A
    /// merge is what closes a Ticket and so unblocks its dependents (ADR
    /// 0002). Gives back each Ticket whose PR's head is quiet, with the
    /// PR's open items not offered before: they are offered now. A Ticket
    /// going back to its PR Stage is polled for its merge or close alone:
    /// nothing starts on it, nor is offered, until /continue takes it back.
    pub(crate) fn poll_merges(&self) -> Vec<(String, Vec<Item>)> {
        let open: Vec<(String, TicketState)> = self
            .state
            .lock()
            .unwrap()
            .tickets
            .iter()
            .filter(|(_, ts)| ts.status == STATUS_PR_OPEN || pr_stage(ts).is_some())
            .map(|(id, ts)| (id.clone(), ts.clone()))
            .collect();
        let (tools, repo) = (&self.cfg.tools, &self.cfg.repo);
        let query = format!("query={}", pr::QUERY);

        let mut quiet = Vec::new();
        for (ticket, ts) in open {
            let url = format!("url={}", ts.pr);
            let reply = tools
                .run(repo, &["gh", "api", "graphql", "-f", &query, "-f", &url])
                .map_err(|err| err.to_string())
                .and_then(|out| pr::parse(&out));
            // read again: it may have parked at its PR Stage during the call
            let ts = self.ticket(&ticket);
            let returning = ts.status != STATUS_PR_OPEN;
            let pr = match reply {
                Ok(pr) => pr,
                Err(err) => {
                    self.log(&ticket, &format!("gh api graphql failed: {err}"));
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
                    ts.stage.clear(); // no PR Stage to go back to
                });
                self.report(
                    &ticket,
                    &format!("parked: {} closed without merging", pr_ref(&ts.pr)),
                );
            } else if !returning {
                // The last poll's view gates /rebase; `conflict` keeps the
                // report, and with rebase_auto the Rebase, to once per
                // conflict: one that fails or parks waits for /rebase, until
                // the PR is seen mergeable again.
                let conflicting = pr.mergeable == "CONFLICTING";
                if conflicting != ts.conflicting {
                    self.update(&ticket, |ts| ts.conflicting = conflicting);
                }
                if conflicting && !ts.conflict {
                    self.update(&ticket, |ts| ts.conflict = true);
                    let auto = app::switch(repo, &app::REBASE_AUTO);
                    let then = if auto {
                        "rebasing it"
                    } else {
                        "/rebase resolves it"
                    };
                    let text = format!("{} conflicts with main, {then}", pr_ref(&ts.pr));
                    self.report(&ticket, &text);
                    if auto {
                        self.command(&format!("rebase-{ticket}"));
                    }
                } else if pr.mergeable == "MERGEABLE" && ts.conflict {
                    self.update(&ticket, |ts| ts.conflict = false);
                }
            }
            if pr.state != "OPEN" || returning {
                continue;
            }

            let now = (self.cfg.clock)();
            if pr.head_ref_oid != ts.head {
                self.update(&ticket, |ts| {
                    ts.head = pr.head_ref_oid.clone();
                    ts.head_at = Some(now);
                });
                continue;
            }
            if pr.busy() || ts.head_at.is_none_or(|at| now - at < QUIET) {
                continue;
            }
            // Rebase goes first: a PR that conflicts, or has a Rebase queued
            // or running, is offered nothing until it is done
            let rebasing = self.active.lock().unwrap().get(&ticket) == Some(&Some(REBASE.name));
            let queued = self.commands().contains(&format!("rebase-{ticket}"));
            if pr.mergeable == "CONFLICTING" || rebasing || queued {
                continue;
            }
            // what is no longer open leaves the set, so it is offered
            // again if it comes back
            let items = pr.items();
            let ids: BTreeSet<String> = items.iter().map(|i| i.id.clone()).collect();
            if ids != ts.offered {
                let mut open = false;
                self.update(&ticket, |ts| {
                    open = ts.status == STATUS_PR_OPEN;
                    if open {
                        ts.offered = ids;
                    }
                });
                if !open {
                    continue;
                }
            }
            let fresh = items.into_iter().filter(|i| !ts.offered.contains(&i.id));
            quiet.push((ticket, fresh.collect()));
        }
        quiet
    }

    /// Runs Rebase for a Ticket whose open PR the last poll saw conflict
    /// with main, on the user's command or, with rebase_auto, the poll's.
    fn rebase(&self, ticket: &str) {
        let ts = self.ticket(ticket);
        if ts.status != STATUS_PR_OPEN {
            return self.report(ticket, "rebase refused: no open PR");
        }
        if !ts.conflicting {
            // a conflict the poll sees next is a new one: said, and with
            // rebase_auto rebased, again
            self.update(ticket, |ts| ts.conflict = false);
            let text = format!("refused: {} does not conflict with main", pr_ref(&ts.pr));
            return self.report(ticket, &text);
        }
        self.on_pr(ticket, &REBASE, false);
    }

    /// Runs Address PR comments for a Ticket with an open PR, on the user's
    /// command only.
    fn address_pr_comments(&self, ticket: &str) {
        if self.ticket(ticket).status != STATUS_PR_OPEN {
            return self.report(ticket, "address pr comments refused: no open PR");
        }
        self.on_pr(ticket, &ADDRESS_PR_COMMENTS, false);
    }

    /// Takes a Ticket back to its PR Stage, never its Pipeline: Parked from
    /// it, or stopped by a long usage limit, and continued.
    fn continue_pr(&self, ticket: &str) {
        if let Some(st) = pr_stage(&self.ticket(ticket)) {
            self.on_pr(ticket, st, true);
        }
    }

    /// Runs a PR Stage in the kept worktree: a fresh session, or `resumed`,
    /// the one /continue takes back. Its tab closes once it is done, said
    /// as "<rebased|addressed> PR #n". Away, its question parks the Ticket;
    /// a long usage limit leaves it running for /continue.
    fn on_pr(&self, ticket: &str, st: &Stage, resumed: bool) {
        let pr = self.ticket(ticket).pr;
        let rebase = st.name == REBASE.name;
        let (name, value) = if rebase {
            ("Default branch", self.origin_head(ticket))
        } else {
            let fields = "reviews,comments,statusCheckRollup";
            let argv = ["gh", "pr", "view", &pr, "--json", fields];
            match self.cfg.tools.run(&self.cfg.repo, &argv) {
                Ok(json) => ("PR metadata (gh JSON)", json.trim().to_string()),
                Err(err) => {
                    let text = format!("address pr comments failed: {err}");
                    // taken back, it stays Parked for the next /continue
                    if !resumed || !self.park_live(ticket, &text) {
                        self.report(ticket, &text);
                    }
                    return;
                }
            }
        };
        let template = if rebase {
            None
        } else {
            self.pr_template(ticket)
        };
        let mut inputs = vec![("PR", pr.as_str()), (name, value.as_str())];
        // Address PR comments keeps the body's sections to the template the
        // last Fix had
        if let Some(template) = &template {
            inputs.push(("PR template", template));
        }
        // the poll may have merged or closed it while the input was fetched:
        // checked under the lock, before the stage it reads is cleared
        let mut live = false;
        self.update(ticket, |ts| {
            live = ts.status == STATUS_PR_OPEN || pr_stage(ts).is_some();
            if !live {
                return;
            }
            if !resumed {
                // a new Stage to run_stage: an earlier session, a parked
                // one's pane still open, is dropped, not watched
                ts.stage.clear();
            }
            ts.status = STATUS_PR_OPEN.to_string();
            ts.reason.clear();
        });
        if !live {
            return;
        }
        if !resumed {
            let _ = fs::remove_file(self.run_dir(ticket).join(result_name(st, 0)));
        }
        let result = self.run_stage(ticket, st, 0, &inputs, ResultRequirements::default());
        match result {
            // a long usage limit ended the run: running, as every Stage
            Err(StageError::Stopped) if self.closed() => {
                self.close_on_limit(ticket);
                self.update(ticket, |ts| {
                    if ts.status == STATUS_PR_OPEN {
                        ts.status = STATUS_RUNNING.to_string();
                    }
                });
            }
            _ if self.stopping() => {}
            Ok(_) => {
                let ts = self.ticket(ticket);
                if !ts.tab.is_empty() {
                    let _ = self.herdr(&["tab", "close", &ts.tab]);
                    self.update(ticket, |ts| {
                        ts.tab.clear();
                        ts.panes.clear();
                        ts.sessions.clear();
                    });
                }
                let done = if rebase { "rebased" } else { "addressed" };
                self.report(ticket, &format!("{done} {}", pr_ref(&ts.pr)));
            }
            Err(StageError::Parked(reason)) => {
                if reason != AWAY || !self.park_live(ticket, &reason) {
                    let text = format!("{} gave up: {reason}", stage_label(st, 0));
                    self.report(ticket, &text);
                }
            }
            Err(StageError::Stopped) => {} // only while stopping, above
        }
    }

    /// Parks a Ticket at its PR Stage unless the poll merged or closed it
    /// meanwhile: checked and parked under one lock. Whether it parked.
    fn park_live(&self, ticket: &str, reason: &str) -> bool {
        let mut live = false;
        self.update(ticket, |ts| {
            live = ts.status == STATUS_PR_OPEN || pr_stage(ts).is_some();
            if live {
                ts.status = STATUS_PARKED.to_string();
                ts.reason = reason.to_string();
            }
        });
        if live {
            self.report(ticket, &format!("parked: {reason}"));
        }
        live
    }
}

/// A Ticket's place in the Pipeline, or its PR session's, given back when
/// its thread ends, also by a panic.
struct Slot {
    o: Arc<Orchestrator>,
    ticket: String,
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.o.active.lock().unwrap().remove(&self.ticket);
    }
}
