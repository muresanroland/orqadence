//! The Release: the Stage a run carrying orqa:release ends in, once every
//! Ticket of the run is merged. It belongs to the run, not to a Ticket: its
//! record is the State's release, apart from the Tickets', and its id
//! (release-<epic>, release-<epic>-<date>-<time> when an earlier Release's
//! worktree has that name, or release-<date>-<time> in a Ticket run, of
//! which a day may have several) names its worktree, Run directory, branch
//! and tab as a Ticket's id names its own.

use std::fs;
use std::time::Instant;

use super::app::{self, AGENT_MERGE, RELEASE_ON};
use super::pipeline::NO_REVIEW_LABEL;
use super::pr::Pr;
use super::result::ResultRequirements;
use super::scheduler::BdIssue;
use super::stage::{pr_ref, result_name, Ask, Orchestrator, StageError, RELEASE};
use super::state::{Release, STATUS_MERGED, STATUS_RUNNING};

/// The Release label: on an Epic, or on any Ticket of a Ticket run, it asks
/// that the run end in a Release.
pub(crate) const RELEASE_LABEL: &str = "orqa:release";

/// Whether bd labels hold the Release label.
pub(crate) fn carries(labels: &[String]) -> bool {
    labels.iter().any(|l| l == RELEASE_LABEL)
}

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
        if !saved
            && !(app::switch(&self.cfg.repo, &RELEASE_ON)
                && children
                    .iter()
                    .any(|c| self.ticket(&c.id).status == STATUS_MERGED))
        {
            return Ok(None);
        }
        if epic.is_empty() {
            let due = saved || children.iter().any(|c| carries(&c.labels));
            return Ok(due.then(|| "none".to_string()));
        }
        let shown = self.bd_all(&["show", epic, "--json"])?;
        let shown = shown.into_iter().next().unwrap_or_default();
        Ok((saved || carries(&shown.labels)).then(|| format!("{epic} {}", shown.title)))
    }

    /// Runs the run's Release on the scheduler's thread, to the run's end:
    /// its Stage, then its version PR and the tag (version_pr), again from
    /// a fresh session when its PR closed unmerged and the user asks for
    /// it. A long usage limit leaves its session saved; a park stops the
    /// run as /stop-work does, the Release saved for /continue.
    pub(super) fn release(&self, epic: &str, epic_input: &str, children: &[BdIssue]) {
        loop {
            let (id, ended) = self.release_stage(epic, epic_input, children);
            match ended.and_then(|()| self.version_pr(&id)) {
                Ok(true) => {}
                Ok(false) => return,
                Err(StageError::Stopped) => return self.close_on_limit(&id),
                Err(StageError::Parked(reason)) => {
                    self.park(&id, &reason);
                    return self.stop();
                }
            }
        }
    }

    /// The Release's Stage, unless its record already has its version: the
    /// record made once, its worktree off origin's default branch, then its
    /// Stage through run_stage, fed the Bump (minor for an Epic, patch for
    /// a Ticket run), the Epic and each merged Ticket with its PR. Done, its
    /// tab closes and its version and PR go into its record. Its id, and how
    /// the Stage ended.
    fn release_stage(
        &self,
        epic: &str,
        epic_input: &str,
        children: &[BdIssue],
    ) -> (String, Result<(), StageError>) {
        let saved = (self.state.lock().unwrap().release.as_ref())
            .map(|r| (r.id.clone(), !r.version.is_empty()));
        let fresh = saved.is_none();
        let bump = if epic.is_empty() { "patch" } else { "minor" };
        let id = match saved {
            Some((id, true)) => return (id, Ok(())),
            Some((id, false)) => id, // a saved one's, from its start
            None => {
                let stamp = (self.cfg.clock)().format("%Y-%m-%d-%H%M%S");
                if epic.is_empty() {
                    format!("release-{stamp}")
                } else if self.worktree(&format!("release-{epic}")).exists() {
                    // an earlier Release's worktree, its branch off an older main
                    format!("release-{epic}-{stamp}")
                } else {
                    format!("release-{epic}")
                }
            }
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
        let want = ResultRequirements {
            require_version: true,
            ..Default::default()
        };
        let ended = self
            .prepare_worktree(&id)
            .and_then(|()| self.run_stage(&id, &RELEASE, 0, &inputs, want));
        let result = match ended {
            Ok(result) => result,
            Err(err) => return (id, Err(err)),
        };
        let tab = self.ticket(&id).tab;
        if !tab.is_empty() {
            let _ = self.herdr(&["tab", "close", &tab]);
        }
        if !result.pr.is_empty() {
            // a No-review pull request whatever its files and whatever
            // agent_merge says (ADR 0007); before its record has the
            // version, so a Release resumed in between labels it again
            let (pr, name) = (pr_ref(&result.pr), NO_REVIEW_LABEL.0);
            match self.add_pr_label(&result.pr, NO_REVIEW_LABEL) {
                Ok(()) => self.log(&id, &format!("version {pr} labelled {name}")),
                Err(err) => self.report("", &format!("version {pr} not labelled {name}: {err}")),
            }
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
        (id, Ok(()))
    }

    /// After the Release's result: its version PR polled until it merges,
    /// then the tag Question, unless a resumed Release is already tagged;
    /// merged by the Orchestrator itself (Agent merge), it is tagged with
    /// no Question, which only a tag that fails then asks. With no version
    /// PR, a repo that keeps its version only in tags, the
    /// Question at once. The PR closed unmerged,
    /// a Question: true to run the Release again, its worktree, branch and
    /// record gone; false to end without one.
    fn version_pr(&self, id: &str) -> Result<bool, StageError> {
        let release = self.release_record();
        let url = &release.ts.pr;
        if !url.is_empty() && release.commit.is_empty() && !self.poll_version_pr(id, url)? {
            let text = format!(
                "version {} closed without merging: run the Release again?",
                pr_ref(url)
            );
            let options = ["run the Release again", "end without a Release"].map(String::from);
            let ask = |options| Ask::ReleaseAgain { options };
            let again = self.ask_at_start(id, &text, options.to_vec(), ask)? == 0;
            if again {
                self.remove_worktree(id);
                self.change_state(|state| state.release = None);
            }
            return Ok(again);
        }
        if !release.tagged {
            // its merge the Orchestrator's own: tagged with no Question
            self.ask_tag(id, !self.ticket(id).merge_asked)?;
        }
        Ok(false)
    }

    /// Polls the version PR on the PRs' interval, as poll_merges polls a
    /// Ticket's, until it merges or closes unmerged: whether it merged.
    /// Merged, its worktree and branch go, as a Ticket's do, and the merge
    /// commit goes into its record. Under Agent merge (ADR 0007) the
    /// Orchestrator merges it once it is ready (version_pr_ready), as it
    /// merges a Ticket's (gh_merge). GitHub refusing is said with gh's
    /// message and not asked again in this run: it waits for a human's
    /// merge, as with Agent merge off.
    fn poll_version_pr(&self, id: &str, url: &str) -> Result<bool, StageError> {
        let mut polled: Option<Instant> = None;
        let mut asked = false;
        loop {
            if polled.is_none_or(|at| at.elapsed() >= self.cfg.poll_prs) {
                polled = Some(Instant::now());
                match self.pr(url) {
                    Err(err) => self.log(id, &format!("gh api graphql failed: {err}")),
                    Ok(pr) if pr.state == "MERGED" => {
                        self.remove_worktree(id);
                        let commit = pr.merge_commit().to_string();
                        self.change_state(|state| {
                            if let Some(release) = &mut state.release {
                                release.commit = commit;
                            }
                        });
                        self.report("", &format!("version {} merged", pr_ref(url)));
                        return Ok(true);
                    }
                    Ok(pr) if pr.state == "CLOSED" => {
                        let text = format!("version {} closed without merging", pr_ref(url));
                        self.report("", &text);
                        return Ok(false);
                    }
                    Ok(pr) if !asked && self.version_pr_ready(id, &pr) => {
                        asked = true;
                        match self.gh_merge(id, url, &pr) {
                            Ok(true) => {
                                self.update(id, |ts| ts.merge_asked = true);
                                let text = format!("version {} merged by Orqadence", pr_ref(url));
                                self.report("", &text);
                            }
                            Ok(false) => asked = false,
                            Err(refused) => {
                                let text = format!("version {} not merged: {refused}", pr_ref(url));
                                self.report("", &text);
                            }
                        }
                    }
                    Ok(_) => {}
                }
            }
            if !self.sleep() {
                return Err(StageError::Stopped);
            }
        }
    }

    /// Whether Agent merge may merge the open version PR now: a No-review
    /// pull request whatever its files, so as a Ticket's (merge) but with
    /// no bot or PR comment to wait for: its head quiet, its checks green,
    /// GitHub calling it mergeable and no review asking for changes.
    fn version_pr_ready(&self, id: &str, pr: &Pr) -> bool {
        app::switch(&self.cfg.repo, &AGENT_MERGE)
            && self.quiet_head(id, &self.ticket(id), pr)
            && pr.green()
            && pr.mergeable == "MERGEABLE"
            && pr.review_decision.as_deref() != Some("CHANGES_REQUESTED")
            && !self.stopping()
    }

    /// The tag Question: yes fetches origin's default branch, tags the
    /// merge commit (with no version PR, that branch's head as fetched) and
    /// pushes the tag alone, never a GitHub Release, which is the repo's own
    /// workflow's; a failure says so and asks again, a tag its push left
    /// deleted so the next git tag can make it. No leaves it to the user:
    /// the same commands in a line and, from the Shell, a Notice. `asked`
    /// false, the first try is made as on a yes, with no Question.
    fn ask_tag(&self, id: &str, mut asked: bool) -> Result<(), StageError> {
        let release = self.release_record();
        let version = release.version.as_str();
        let target = if release.ts.pr.is_empty() {
            "FETCH_HEAD"
        } else {
            &release.commit
        };
        let cmds = [
            ["git", "fetch", "origin", "HEAD"],
            ["git", "tag", version, target],
            ["git", "push", "origin", version],
        ];
        let lines = cmds.map(|argv| argv.join(" "));
        let [fetch, tag, push] = &cmds;
        let notice = format!(
            "{version} not tagged. To tag it and push the tag:\n\n{}",
            lines.join("\n")
        );
        let text = format!("Tag {version} and push it?");
        let run = |argv: &[&str]| self.cfg.tools.run(&self.cfg.repo, argv).map(drop);
        loop {
            let options = vec!["yes".to_string(), "no".to_string()];
            let ask = |options| Ask::Tag {
                options,
                notice: notice.clone(),
            };
            if asked && self.ask_at_start(id, &text, options, ask)? == 1 {
                let how = lines.join(" && ");
                self.report("", &format!("{version} not tagged: {how}"));
                return Ok(());
            }
            let tagged = run(fetch)
                .and_then(|()| run(tag))
                .and_then(|()| run(push).inspect_err(|_| _ = run(&["git", "tag", "-d", version])));
            match tagged {
                Ok(()) => {
                    self.change_state(|state| {
                        if let Some(release) = &mut state.release {
                            release.tagged = true;
                        }
                    });
                    self.report("", &format!("tagged {version} and pushed"));
                    return Ok(());
                }
                Err(err) => self.report("", &format!("tag {version} failed: {err}")),
            }
            asked = true;
        }
    }

    /// A snapshot of the run's Release record.
    fn release_record(&self) -> Release {
        let state = self.state.lock().unwrap();
        state.release.as_deref().cloned().unwrap_or_default()
    }
}
