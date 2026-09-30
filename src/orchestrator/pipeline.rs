//! The Pipeline: Implement, then Rounds of Review, Debate and Fix until a
//! Verdict has no fix items or the cap, then the pull request.

use std::fs;
use std::path::Path;

use serde_json::json;

use super::app::{self, ExtraReview};
use super::result::{read_stage_result, ResultRequirements, StageResult};
use super::stage::{
    plural, pr_ref, result_name, stage_label, Ask, Orchestrator, Stage, StageError, ADDRESS, AWAY,
    DEBATE, EXTRA_REVIEW, FINAL, FIX, IMPLEMENT, REVIEW,
};
use super::state::{STATUS_PARKED, STATUS_PR_OPEN, STATUS_RUNNING};
use crate::skills::manifest::{placeholder, unlink_checkout_skills, Manifest, FILES, JOBS, LINKS};
use crate::skills::{stage_skill, CREATE_PR};

pub(crate) const MAX_ROUNDS: usize = 3;

/// What a run keeps for whoever reads it later: the Stages' result files,
/// diffs and debate transcripts, all flat text.
const EVIDENCE: [&str; 5] = ["md", "txt", "patch", "json", "sh"];

/// The Fix Stage's Input of fix items: "none", or one per line.
fn fix_items(fixes: &[String]) -> String {
    if fixes.is_empty() {
        "none".to_string()
    } else {
        format!("\n  {}", fixes.join("\n  "))
    }
}

/// An Extra review's Findings as fix items that skipped the Debate.
fn not_debated_items(found: &StageResult) -> Vec<String> {
    let mark = |item: &String| format!("- [fix] {} | not debated | extra review", &item[2..]);
    found.found.iter().map(mark).collect()
}

impl Orchestrator {
    /// Moves one Ticket through the Pipeline: Implement, then Rounds of
    /// Review, Debate and Fix until a Verdict has no fix items or the cap is
    /// reached, ending with an open pull request. Finished Stages are skipped
    /// by their result files, so calling it again resumes where a stopped run
    /// stopped.
    pub(crate) fn run_ticket(&self, ticket: &str) {
        match self.pipeline(ticket) {
            Ok(()) => {}
            Err(StageError::Stopped) => self.close_on_limit(ticket),
            Err(StageError::Parked(reason)) => {
                self.update(ticket, |ts| {
                    ts.status = STATUS_PARKED.to_string();
                    ts.reason = reason.clone();
                });
                self.report(ticket, &format!("parked: {reason}"));
            }
        }
    }

    fn pipeline(&self, ticket: &str) -> Result<(), StageError> {
        self.update(ticket, |ts| {
            ts.status = STATUS_RUNNING.to_string();
            ts.reason.clear();
        });
        self.prepare_worktree(ticket)?;
        self.ask_unmerged_picks(ticket)?;
        // labels that cannot be read Wake the Stage that reads them
        self.ask_labels(ticket, &mut self.labels(ticket).unwrap_or_default())?;
        self.ask_shadowed(ticket)?;

        self.run_stage(ticket, &IMPLEMENT, 0, &[], ResultRequirements::default())?;
        self.report(ticket, "implemented");

        let mut verdicts = Vec::new();
        for round in 1..=MAX_ROUNDS {
            let review_file = self.run_dir(ticket).join(result_name(&REVIEW, round));
            let review =
                self.run_read_only(ticket, &REVIEW, round, &[], ResultRequirements::default())?;
            // The Area label's Extra review, after the Review in every Round
            // or in Round 1 only; one before the PR runs once, after the
            // last Round's Fix.
            let labels = self
                .labels(ticket)
                .map_err(|err| StageError::Parked(format!("Ticket labels not read: {err}")))?;
            let extra = app::extra_review(&self.cfg.repo, &labels).map_err(StageError::Parked)?;
            let (before_pr, extra) = match extra {
                Some(extra) if extra.position == "before_pr" => (Some(extra), None),
                extra => (
                    None,
                    extra.filter(|extra| extra.position != "first" || round == 1),
                ),
            };
            // Its App Limited and the PR to open unreviewed: this Round's
            // Review, Extra review and Debate are skipped, and nothing is
            // left to fix.
            let mut unreviewed = review.unreviewed;
            let fixes = if !unreviewed.is_empty() {
                self.report(
                    ticket,
                    &format!("review {round} and debate {round} skipped: {unreviewed}"),
                );
                let skipped = match (&extra, &before_pr) {
                    (Some(_), _) => Some(format!("extra review {round}")),
                    (_, Some(_)) => Some("extra review before the PR".to_string()),
                    _ => None,
                };
                if let Some(skipped) = skipped {
                    self.report(ticket, &format!("{skipped} skipped: {unreviewed}"));
                    unreviewed += ", the extra review skipped too";
                }
                Vec::new()
            } else {
                self.report(
                    ticket,
                    &format!(
                        "review {round} found {}",
                        plural(review.found.len(), "finding")
                    ),
                );
                let review_file = review_file.display().to_string();
                let extra_file = self
                    .run_dir(ticket)
                    .join(result_name(&EXTRA_REVIEW, round))
                    .display()
                    .to_string();
                let mut inputs = vec![("Review file", review_file.as_str())];
                let mut findings = review.found.len();
                // Debate off: its Findings are fix items, not debated.
                let mut not_debated = Vec::new();
                if let Some(extra) = &extra {
                    let found = self.run_read_only(
                        ticket,
                        &EXTRA_REVIEW,
                        round,
                        &[],
                        ResultRequirements::default(),
                    )?;
                    self.report(
                        ticket,
                        &format!(
                            "extra review {round} found {}",
                            plural(found.found.len(), "finding")
                        ),
                    );
                    if extra.debate {
                        inputs.push(("Extra review file", extra_file.as_str()));
                        findings += found.found.len();
                    } else {
                        not_debated = not_debated_items(&found);
                    }
                }
                let verdict = self.run_read_only(
                    ticket,
                    &DEBATE,
                    round,
                    &inputs,
                    ResultRequirements {
                        review_findings: findings,
                        ..Default::default()
                    },
                )?;
                self.report(
                    ticket,
                    &format!(
                        "debate {round} settled: {} to fix, {} skipped",
                        verdict.fixes.len(),
                        verdict.skips.len()
                    ),
                );
                verdicts.push(
                    self.run_dir(ticket)
                        .join(result_name(&DEBATE, round))
                        .display()
                        .to_string(),
                );
                // the Extra review's, after the Verdict's: they keep the
                // Rounds going too
                let mut fixes = verdict.fixes;
                fixes.extend(not_debated);
                fixes
            };

            // The Fix session always runs, even with nothing to fix, because
            // the last one opens the pull request. It is given only the fix
            // items; the last one also gets the Verdict files, for the PR
            // description. After a reviewed last Round, an Extra review
            // before the PR holds the PR for a final Fix.
            let last = fixes.is_empty() || round == MAX_ROUNDS;
            let held = last && unreviewed.is_empty() && before_pr.is_some();
            let items = fix_items(&fixes);
            let history = verdicts.join(", ");
            let mut inputs = vec![("Open PR", "no"), ("Fix items", items.as_str())];
            if last && !held {
                inputs[0].1 = "yes";
                inputs.push(("Verdict history", history.as_str()));
            }
            if !unreviewed.is_empty() {
                inputs.push(("Unreviewed", unreviewed.as_str()));
            }
            let mut fix = self.run_stage(
                ticket,
                &FIX,
                round,
                &inputs,
                ResultRequirements {
                    require_pr: last && !held,
                    ..Default::default()
                },
            )?;
            self.report(ticket, &format!("fix {round} done"));
            if !last {
                continue;
            }
            if let Some(extra) = before_pr.filter(|_| held) {
                fix = self.final_fix(ticket, &extra, &mut verdicts)?;
            }

            let tab = self.ticket(ticket).tab;
            self.update(ticket, |ts| {
                ts.status = STATUS_PR_OPEN.to_string();
                ts.pr = fix.pr.clone();
                ts.tab.clear();
                ts.panes.clear();
                ts.sessions.clear();
            });
            if !tab.is_empty() {
                let _ = self.herdr(&["tab", "close", &tab]);
            }
            self.emit(
                ticket,
                &format!(
                    "{} opened after {}",
                    pr_ref(&fix.pr),
                    plural(round, "round")
                ),
                true,
                &fix.pr, // the log line adds the url
            );
            self.wait_dependents(ticket, &fix.pr);
            self.prune_run_dir(ticket);
            return Ok(());
        }
        Ok(())
    }

    /// What follows a last Round's Fix that held the PR: the Extra review
    /// once on the finished branch, its Debate when its switch is on (off,
    /// its Findings are fix items not debated), and a final Fix with the
    /// Verdict history, which opens the PR even with no fix items.
    fn final_fix(
        &self,
        ticket: &str,
        extra: &ExtraReview,
        verdicts: &mut Vec<String>,
    ) -> Result<StageResult, StageError> {
        let dir = self.run_dir(ticket);
        let file = dir.join(result_name(&EXTRA_REVIEW, FINAL));
        let found = self.run_read_only(
            ticket,
            &EXTRA_REVIEW,
            FINAL,
            &[],
            ResultRequirements::default(),
        )?;
        self.report(
            ticket,
            &format!(
                "extra review before the PR found {}",
                plural(found.found.len(), "finding")
            ),
        );
        let fixes = if extra.debate {
            let file = file.display().to_string();
            let verdict = self.run_read_only(
                ticket,
                &DEBATE,
                FINAL,
                &[("Review file", file.as_str())],
                ResultRequirements {
                    review_findings: found.found.len(),
                    ..Default::default()
                },
            )?;
            self.report(
                ticket,
                &format!(
                    "debate final settled: {} to fix, {} skipped",
                    verdict.fixes.len(),
                    verdict.skips.len()
                ),
            );
            verdicts.push(dir.join(result_name(&DEBATE, FINAL)).display().to_string());
            verdict.fixes
        } else {
            not_debated_items(&found)
        };
        let (items, history) = (fix_items(&fixes), verdicts.join(", "));
        let inputs = [
            ("Open PR", "yes"),
            ("Fix items", items.as_str()),
            ("Verdict history", history.as_str()),
        ];
        let fix = self.run_stage(
            ticket,
            &FIX,
            FINAL,
            &inputs,
            ResultRequirements {
                require_pr: true,
                ..Default::default()
            },
        )?;
        self.report(ticket, "final fix done");
        Ok(fix)
    }

    /// Runs a Stage that must leave the worktree as it found it: the Review
    /// and the Debate, whatever their App (not every App has a sandbox). The
    /// HEAD and tree before it are saved in the Run directory once, before
    /// the Stage first runs, so a resumed or retried Stage, or one parked by
    /// its guard and continued, is compared with the tree it started from. A
    /// Stage already done with no snapshot is not guarded: the tree may hold
    /// a later Stage's work. One parked for asking while the user was Away
    /// keeps its session and snapshot: /continue @ticket watches it again.
    fn run_read_only(
        &self,
        ticket: &str,
        st: &Stage,
        round: usize,
        inputs: &[(&str, &str)],
        want: ResultRequirements,
    ) -> Result<StageResult, StageError> {
        let label = stage_label(st, round);
        let snapshot = self
            .run_dir(ticket)
            .join(format!("before-{}-{round}.json", st.name));
        let file = self.run_dir(ticket).join(result_name(st, round));
        let done = read_stage_result(&file, want).1.is_empty();
        if !done && !snapshot.exists() {
            if let Some(head) = self.head(ticket) {
                let before = json!([head, self.tree(ticket)]).to_string();
                fs::write(&snapshot, before).map_err(|err| {
                    StageError::Parked(format!("{label}: worktree snapshot not saved: {err}"))
                })?;
            }
        }
        let result = self.run_stage(ticket, st, round, inputs, want);
        let parked = match &result {
            Err(StageError::Parked(reason)) if reason != AWAY => Some(reason),
            _ => None,
        };
        if let Some(reason) = parked {
            // a parked Ticket may never continue: end its session, which
            // could still write, and put its tree back now. Once back, the
            // snapshot goes: a parked tree is the user's to edit, and a
            // continued Stage starts fresh and is compared with what they
            // left. A tree not put back keeps it, and the reason says so
            if let Some(pane) = self.ticket(ticket).panes.get(st.name) {
                let _ = self.herdr(&["pane", "close", pane]);
                self.update(ticket, |ts| {
                    ts.panes.remove(st.name);
                    ts.sessions.remove(st.name);
                });
            }
            if let Err(StageError::Parked(kept)) = self.guard(ticket, &label, &snapshot) {
                return Err(StageError::Parked(format!("{reason}; {kept}")));
            }
            let _ = fs::remove_file(&snapshot);
        }
        let result = result?;
        self.guard(ticket, &label, &snapshot)?;
        let _ = fs::remove_file(&snapshot);
        Ok(result)
    }

    /// The worktree as `[head, tree]` JSON, what a snapshot file holds.
    pub(super) fn snapshot(&self, ticket: &str) -> String {
        json!([self.head(ticket), self.tree(ticket)]).to_string()
    }

    /// The worktree's HEAD; None when git cannot say.
    fn head(&self, ticket: &str) -> Option<String> {
        let argv = ["git", "rev-parse", "HEAD"];
        let head = self.cfg.tools.run(&self.worktree(ticket), &argv).ok()?;
        Some(head.trim().to_string())
    }

    /// The worktree beyond its HEAD: `git status --porcelain`, the content
    /// of each change to a tracked file and each untracked file's path and
    /// hash, so an edit to a file already changed, or a rename inside an
    /// untracked directory, shows too. Empty for a clean tree;
    /// None when git cannot say.
    fn tree(&self, ticket: &str) -> Option<String> {
        let (tools, worktree) = (&self.cfg.tools, self.worktree(ticket));
        let git = |argv: &[&str]| tools.run(&worktree, argv).ok();
        let status = git(&["git", "status", "--porcelain"])?;
        let diff = git(&["git", "diff", "HEAD", "--binary"])?;
        let untracked = git(&["git", "ls-files", "--others", "--exclude-standard", "-z"])?;
        let mut hash = vec!["git", "hash-object", "--"];
        hash.extend(untracked.split('\0').filter(|path| !path.is_empty()));
        let hashes = match untracked.is_empty() {
            true => String::new(),
            false => git(&hash)?,
        };
        Some(
            format!("{status}{diff}{untracked}{hashes}")
                .trim()
                .to_string(),
        )
    }

    /// Puts back a worktree the Stage changed: a moved HEAD or a changed
    /// tree is reset to the HEAD and clean tree in its snapshot. A tree
    /// already dirty before the Stage holds work that is not the Stage's
    /// (Implement's, a user's), so it is never reset: a change to it parks
    /// the Ticket, as does a tree that cannot be put back, since Fix would
    /// commit it. The snapshot is kept: the caller drops it once the tree
    /// matches or is put back, so a Ticket parked here is guarded again.
    fn guard(&self, ticket: &str, label: &str, snapshot: &Path) -> Result<(), StageError> {
        let Some((head, tree)) = fs::read(snapshot)
            .ok()
            .and_then(|raw| serde_json::from_slice::<(String, Option<String>)>(&raw).ok())
        else {
            return Ok(());
        };
        let now = self.tree(ticket);
        if now.is_some() && now == tree && self.head(ticket).as_ref() == Some(&head) {
            return Ok(());
        }
        if tree.as_deref() != Some("") {
            return Err(StageError::Parked(format!(
                "{label} changed a worktree already dirty before it, not restored"
            )));
        }
        let (tools, worktree) = (&self.cfg.tools, self.worktree(ticket));
        // -fd, not -fdx: ignored build caches stay
        tools
            .run(&worktree, &["git", "reset", "--hard", &head])
            .and_then(|_| tools.run(&worktree, &["git", "clean", "-fd"]))
            .map_err(|err| {
                StageError::Parked(format!("{label} changed the worktree, not restored: {err}"))
            })?;
        self.report(ticket, &format!("{label} changed the worktree: restored"));
        Ok(())
    }

    /// Drops a Ticket's build scratch once its pull request is open and its
    /// Pipeline is over. The run directory is the Codex sandbox's only
    /// writable root, so a Stage that has to compile puts its build cache
    /// there: a Go cache runs to some 100MB per Ticket, and nothing reads it
    /// again. Keeping only the evidence survives the next Stage inventing a
    /// fifth name for its cache. Best effort: scratch that cannot be removed
    /// is only disk.
    fn prune_run_dir(&self, ticket: &str) {
        let dir = self.run_dir(ticket);
        let Ok(entries) = fs::read_dir(&dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let evidence = path
                .extension()
                .is_some_and(|ext| EVIDENCE.contains(&ext.to_string_lossy().as_ref()));
            if path.is_dir() || !evidence {
                let removed = if path.is_dir() {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                };
                if let Err(err) = removed {
                    self.log(ticket, &format!("scratch left in the run directory: {err}"));
                }
            }
        }
    }

    /// Creates the Ticket's worktree and branch once, brings the new branch
    /// up to the remote's default branch so a dependent Ticket builds on what
    /// was just merged (ADR 0002), and marks the Ticket in progress. It
    /// gets no skill links: its Stages run the skills committed on its base
    /// (ADR 0006), so each time those an older Orqadence linked in go.
    fn prepare_worktree(&self, ticket: &str) -> Result<(), StageError> {
        let worktree = self.worktree(ticket);
        let tools = &self.cfg.tools;
        let repo = &self.cfg.repo;
        if !worktree.exists() {
            let path = worktree.display().to_string();
            tools
                .run(
                    repo,
                    &["bd", "worktree", "create", &path, "--branch", ticket],
                )
                .map_err(|err| StageError::Parked(format!("worktree not created: {err}")))?;
            if let Err(err) = tools.run(&worktree, &["git", "pull", "--ff-only", "origin", "HEAD"])
            {
                // Building on a stale main is what ADR 0002 exists to prevent;
                // leave nothing behind so a retry prepares the worktree again.
                let _ = tools.run(repo, &["bd", "worktree", "remove", &path]);
                let _ = tools.run(repo, &["git", "branch", "-D", ticket]);
                return Err(StageError::Parked(format!(
                    "new branch not brought up to origin's default branch: {err}"
                )));
            }
            if let Err(err) = tools.run(repo, &["bd", "update", ticket, "--status", "in_progress"])
            {
                self.log(ticket, &format!("not marked in_progress: {err}"));
            }
            self.report(ticket, &format!("branch {ticket} created"));
        }
        unlink_checkout_skills(repo, &worktree).map_err(|err| {
            StageError::Parked(format!(
                "old link to the checkout's skills not removed: {err}"
            ))
        })
    }

    /// A skill the checkout's Skill manifest records as installed but the
    /// Ticket's worktree, a checkout of its base, lacks was added in /config
    /// and not yet merged: a Question before any Stage, one per skill. A
    /// job's pick, or a skill of the Ticket's labels (its label skills and
    /// its Extra review's). Run without it, it is left out, as a skill not
    /// installed is, even with a personal skill of its name (attempt's
    /// have): a pick's line, a label skill's place under Label skills.
    /// Personal, plugin and built-in picks are not the manifest's.
    /// Asked at the start alone: once a Stage has run, the branch holds the
    /// Ticket's work and no longer takes the base as it moves.
    fn ask_unmerged_picks(&self, ticket: &str) -> Result<(), StageError> {
        if !self.ticket(ticket).stage.is_empty() {
            return Ok(());
        }
        // one that cannot be read Wakes the Stage that loads it
        let Ok(manifest) = Manifest::load(&self.cfg.repo) else {
            return Ok(());
        };
        let worktree = self.worktree(ticket);
        // (skill, the Question, ": " and what run without leaves out, or "")
        let mut missing: Vec<(String, String, String)> = Vec::new();
        for (job, _) in JOBS {
            let pick = manifest.pick(job);
            if manifest.unmerged(&worktree, pick) {
                missing.push((
                    pick.to_string(),
                    format!(
                        "{pick}, picked for {job}, is not on this Ticket's base branch: \
                         added in /config and not yet merged"
                    ),
                    format!(": the {job} line is left out"),
                ));
            }
        }
        // labels that cannot be read Wake the Stage that reads them
        let labels = self.labels(ticket).unwrap_or_default();
        let entries = app::ticket_labels(&self.cfg.repo, &labels).unwrap_or_default();
        for (_, label) in &entries {
            let extra = Some(&label.extra_review.skill).filter(|s| !s.is_empty());
            for skill in label.skills.iter().chain(extra) {
                let asked = missing.iter().any(|(name, ..)| name == skill);
                if !asked && manifest.unmerged(&worktree, skill) {
                    missing.push((
                        skill.clone(),
                        format!(
                            "{skill} is not committed on {ticket}'s base: commit and merge it \
                             first, or run without it"
                        ),
                        String::new(),
                    ));
                }
            }
        }
        if missing.is_empty() {
            return Ok(());
        }
        // Merged since a park, say: the branch, still the base's, is
        // brought up to it. Not brought up, the worktree says nothing about
        // the base: park rather than ask or build on a stale one.
        let pull = ["git", "pull", "--ff-only", "origin", "HEAD"];
        if let Err(err) = self.cfg.tools.run(&worktree, &pull) {
            return Err(StageError::Parked(format!(
                "branch not brought up to origin's default branch: {err}"
            )));
        }
        for (skill, text, left_out) in missing {
            if !manifest.unmerged(&worktree, &skill) {
                continue;
            }
            let options = vec![
                format!("park: commit and merge {skill}, then /continue @{ticket}"),
                format!("run without it{left_out}"),
            ];
            let picked = self.ask_at_start(ticket, &text, options, |options| Ask::TicketStart {
                options,
            });
            if picked? == 0 {
                return Err(StageError::Parked(format!(
                    "{skill} not merged: commit and merge it, then /continue @{ticket}"
                )));
            }
            self.report(ticket, &format!("running without {skill}{left_out}"));
        }
        Ok(())
    }

    /// A personal skill with the name of a committed one a Stage loads by
    /// name (each job's pick, orqa-create-pr for the Fix, the review pick on
    /// review_if_limited's row too while it is set, the Ticket's label skills
    /// on the code-editing Stages' rows), in a home folder of
    /// the App on the row that loads it (home_skills), shadows it: claude
    /// runs the personal one, codex may. A Question each, before any Stage
    /// on each entry, but for Implement's jobs once it is done: gone on
    /// with, it stands for the run (shadows); parked, /continue asks again.
    /// A pick not committed (a personal, plugin or built-in one, or one not
    /// merged) has nothing to shadow, nor has one whose placeholder no
    /// committed Stage skill holds: no Stage loads it.
    fn ask_shadowed(&self, ticket: &str) -> Result<(), StageError> {
        let (repo, home) = (&self.cfg.repo, &self.cfg.home);
        // one that cannot be read Wakes the Stage that loads it
        let Ok(manifest) = Manifest::load(repo) else {
            return Ok(());
        };
        if home.as_os_str().is_empty() {
            return Ok(());
        }
        let worktree = self.worktree(ticket);
        // labels that cannot be read Wake the Stage that reads them
        let labels = self.labels(ticket).unwrap_or_default();
        let entries = app::ticket_labels(repo, &labels).unwrap_or_default();
        let implement = self.run_dir(ticket).join(result_name(&IMPLEMENT, 0));
        let implemented = read_stage_result(&implement, ResultRequirements::default())
            .1
            .is_empty();
        // Each Stage skill's placeholders on the row whose App fills them
        // in and runs the Stage: the Debate's on side A's.
        let rows = [
            (&IMPLEMENT, IMPLEMENT.name),
            (&REVIEW, REVIEW.name),
            (&REVIEW, app::IF_LIMITED),
            (&DEBATE, "side_a"),
            (&FIX, FIX.name),
            (&ADDRESS, ADDRESS.name),
        ];
        let mut loaded: Vec<(&str, &str)> = Vec::new();
        for (st, key) in rows {
            if implemented && st.name == IMPLEMENT.name {
                continue;
            }
            let Some(Ok(skill)) = stage_skill(&worktree, st.skill) else {
                continue;
            };
            let held = JOBS
                .iter()
                .filter(|(job, _)| skill.contains(&placeholder(job)));
            loaded.extend(held.map(|(job, _)| (manifest.pick(job), key)));
        }
        loaded.push((CREATE_PR, FIX.name));
        // the label skills, on the code-editing Stages' rows
        for (_, label) in &entries {
            for skill in &label.skills {
                for st in [&IMPLEMENT, &FIX, &ADDRESS] {
                    if !(implemented && st.name == IMPLEMENT.name) {
                        loaded.push((skill, st.name));
                    }
                }
            }
        }
        for (name, key) in loaded {
            // a row that cannot be read is its Stage's to refuse
            let row = match key {
                app::IF_LIMITED => app::fallback_row(repo, &labels).ok().flatten(),
                _ => app::row(repo, key, &labels).ok(),
            };
            let Some(row) = row else {
                continue;
            };
            // committed where the row's App loads it; one not merged is
            // left out of the Stage, whatever older copy the base holds
            let committed = LINKS
                .into_iter()
                .chain([FILES])
                .filter(|dir| row.app.loads(name, Path::new(dir)))
                .map(|dir| worktree.join(dir).join(name).join("SKILL.md"))
                .find(|path| path.exists());
            let Some(committed) = committed.filter(|_| !manifest.unmerged(&worktree, name)) else {
                continue;
            };
            let committed = fs::read(committed).ok();
            for dir in row.app.home_skills {
                let yours = format!("~/{dir}/{name}");
                // the same SKILL.md as the committed one runs the same, whichever
                // ponytail: SKILL.md alone; compare the folders if one differs in its other files
                let personal = fs::read(home.join(dir).join(name).join("SKILL.md")).ok();
                if personal.is_none()
                    || personal == committed
                    || !self.claim_shadow(ticket, &yours)?
                {
                    continue;
                }
                let runs = if row.app.name == "claude" {
                    "runs"
                } else {
                    "may run"
                };
                let text = format!(
                    "your {yours} shadows the committed {name}: {} {runs} yours",
                    row.app.name
                );
                let options = vec![
                    "go on with yours".to_string(),
                    format!("park: rename yours, then /continue @{ticket}"),
                ];
                let picked = self.ask_at_start(ticket, &text, options, |options| {
                    Ask::TicketStart { options }
                });
                let mut shadows = self.shadows.lock().unwrap();
                match picked {
                    Ok(0) => shadows.insert(yours.clone(), true),
                    _ => shadows.remove(&yours), // another Ticket may ask
                };
                drop(shadows);
                if picked? == 1 {
                    return Err(StageError::Parked(format!(
                        "rename your {yours}, then /continue @{ticket}"
                    )));
                }
                self.report(ticket, &format!("going on with your {yours}"));
            }
        }
        Ok(())
    }

    /// Whether this Ticket is to ask about the personal skill `yours`: it
    /// takes the Question when none is out; while another Ticket's is, it
    /// waits for that answer, which is its own too; gone on with, nothing.
    /// /park parks it, /stop-work stops it.
    fn claim_shadow(&self, ticket: &str, yours: &str) -> Result<bool, StageError> {
        loop {
            let mut shadows = self.shadows.lock().unwrap();
            match shadows.get(yours) {
                Some(true) => return Ok(false),
                Some(false) => {}
                None => {
                    shadows.insert(yours.to_string(), false);
                    return Ok(true);
                }
            }
            drop(shadows);
            self.wait_at_start(ticket)?;
        }
    }
}
