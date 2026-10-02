//! The Pipeline: Implement, then Rounds of Review, Debate and Fix until a
//! Verdict has no fix items or the cap, then the pull request.

use std::fs;
use std::path::Path;

use serde_json::json;

use super::app::{self, ExtraReview};
use super::result::{read_stage_result, ResultRequirements, StageResult};
use super::stage::{
    plural, pr_ref, result_name, stage_label, Ask, Orchestrator, Stage, StageError,
    ADDRESS_PR_COMMENTS, AWAY, DEBATE, EDITING, EXTRA_REVIEW, FINAL, FIX, IMPLEMENT, REBASE,
    REVIEW,
};
use super::state::{local_dir, TicketState, LOCAL, STATUS_PARKED, STATUS_PR_OPEN, STATUS_RUNNING};
use crate::setup::{DEFAULT_TEMPLATE, TEMPLATE_DIR};
use crate::skills::manifest::{placeholder, unlink_checkout_skills, Manifest, FILES, JOBS, LINKS};
use crate::skills::{stage_skill, CREATE_PR};
use crate::tools::RunError;

pub(crate) const MAX_ROUNDS: usize = 3;

/// What a run keeps for whoever reads it later: the Stages' result files,
/// diffs and debate transcripts, all flat text.
const EVIDENCE: [&str; 5] = ["md", "txt", "patch", "json", "sh"];

/// A failed fetch.sh's error: the last three lines of its stderr on one
/// line, or how it failed when it wrote none.
fn fetch_error(err: &RunError) -> String {
    let lines: Vec<&str> = err
        .stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    match lines.len() {
        0 => err.status.clone(),
        n => lines[n.saturating_sub(3)..].join(" / "),
    }
}

/// An Extra review's Findings as fix items that skipped the Debate.
fn not_debated_items(found: &StageResult) -> Vec<String> {
    let mark = |item: &String| format!("- [fix] {} | not debated | extra review", &item[2..]);
    found.found.iter().map(mark).collect()
}

/// A GitHub label a Ticket's PR carries: its name, and the colour and
/// description it is created with.
type GhLabel = (&'static str, &'static str, &'static str);
const HUMAN_MERGE_LABEL: GhLabel = (
    "orqa:human-merge",
    "D93F0B",
    "A human merges this pull request, never Orqadence",
);
const NO_REVIEW_LABEL: GhLabel = (
    "orqa:no-review",
    "C5DEF5",
    "Only Markdown and skills changed: the review bots skip it",
);

/// Where a skill's files live, from the repo's root.
const SKILL_DIRS: [&str; 4] = [
    "skills/",
    ".orqadence/skills/",
    ".claude/skills/",
    ".agents/skills/",
];

/// Whether a PR changing these paths, each ended by a NUL as `git diff -z`
/// prints them, is a No-review pull request: each is Markdown or a skill's
/// file. A fixed rule no agent judges; no path at all says nothing, so it
/// is not one.
fn is_no_review(paths: &str) -> bool {
    !paths.is_empty()
        && paths
            .split_terminator('\0')
            .all(|path| path.ends_with(".md") || SKILL_DIRS.iter().any(|d| path.starts_with(d)))
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
            Err(StageError::Parked(reason)) => self.park(ticket, &reason),
        }
    }

    /// Takes the Ticket out to wait for the user, saying why.
    pub(super) fn park(&self, ticket: &str, reason: &str) {
        self.update(ticket, |ts| {
            ts.status = STATUS_PARKED.to_string();
            ts.reason = reason.to_string();
        });
        self.report(ticket, &format!("parked: {reason}"));
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
                    let found = self.run_extra_review(ticket, round)?;
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
                let mut fixes = self.debate(ticket, round, &inputs, findings, &mut verdicts)?;
                // the Extra review's, after the Verdict's: they keep the
                // Rounds going too
                fixes.extend(not_debated);
                fixes
            };

            // The Fix session always runs, even with nothing to fix, because
            // the last one opens the pull request. It is given only the fix
            // items; the last one also gets the Verdict files, for the PR
            // description. After a reviewed last Round, an Extra review
            // before the PR holds the PR for a final Fix.
            let last = fixes.is_empty() || round == MAX_ROUNDS;
            let held = before_pr.filter(|_| last && unreviewed.is_empty());
            let mut fix = self.fix(
                ticket,
                round,
                &fixes,
                last && held.is_none(),
                &unreviewed,
                &verdicts,
            )?;
            if !last {
                continue;
            }
            if let Some(extra) = held {
                fix = self.final_fix(ticket, &extra, &mut verdicts)?;
            }

            let (human_merge, no_review) = self.label_pr(ticket, &fix.pr);
            let tab = self.ticket(ticket).tab;
            self.update(ticket, |ts| {
                ts.status = STATUS_PR_OPEN.to_string();
                ts.pr = fix.pr.clone();
                ts.human_merge = human_merge;
                ts.no_review = no_review;
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
            self.prune_run_dir(ticket);
            return Ok(());
        }
        Ok(())
    }

    /// Labels the PR the last Fix opened, whatever agent_merge says, and
    /// gives back whether it is human-merge and whether it is a No-review
    /// pull request, for the Ticket's state. Human-merge wins: such a PR is
    /// never No-review. Labels or a diff that cannot be read fall on the
    /// safe side, a human merging and the bots reviewing, and RECENT says
    /// so; gh failing leaves a line there too, and a PR without its
    /// orqa:no-review is not No-review, since the bots review it.
    fn label_pr(&self, ticket: &str, pr: &str) -> (bool, bool) {
        let human_merge = self.is_human_merge(ticket, pr);
        let label = if human_merge {
            HUMAN_MERGE_LABEL
        } else if self.changes_no_review(ticket, pr, "HEAD") {
            NO_REVIEW_LABEL
        } else {
            return (false, false);
        };
        let (pr_ref, name) = (pr_ref(pr), label.0);
        let labelled = self.add_pr_label(pr, label);
        match &labelled {
            Ok(()) => self.log(ticket, &format!("{pr_ref} labelled {name}")),
            Err(err) => self.report(ticket, &format!("{pr_ref} not labelled {name}: {err}")),
        }
        (human_merge, !human_merge && labelled.is_ok())
    }

    /// Whether the Ticket's labels make its PR human-merge; labels that
    /// cannot be read do, and RECENT says so.
    fn is_human_merge(&self, ticket: &str, pr: &str) -> bool {
        self.labels(ticket)
            .and_then(|names| Ok(app::human_merge(&app::read(&self.cfg.repo)?.1, &names)))
            .unwrap_or_else(|err| {
                let text = format!(
                    "Ticket labels not read, so a human merges {}: {err}",
                    pr_ref(pr)
                );
                self.report(ticket, &text);
                true
            })
    }

    /// Whether the Ticket's branch, from its base to `head`, changed only
    /// Markdown and skills; a diff that cannot be read did not, and RECENT
    /// says so.
    fn changes_no_review(&self, ticket: &str, pr: &str, head: &str) -> bool {
        // --no-renames: a source file renamed to Markdown is a source file
        // deleted. -z, so a path git would quote comes as it is.
        let range = format!("{}...{head}", self.origin_head(ticket));
        let argv = ["git", "diff", "--no-renames", "--name-only", "-z", &range];
        match self.cfg.tools.run(&self.worktree(ticket), &argv) {
            Ok(paths) => is_no_review(&paths),
            Err(err) => {
                let text = format!(
                    "changed files not read, so {} is reviewed: {err}",
                    pr_ref(pr)
                );
                self.report(ticket, &text);
                false
            }
        }
    }

    /// Reads again, before a merge, what label_pr read as the PR opened:
    /// the Ticket's labels, and the diff to the head about to merge. A
    /// state saved before human_merge was, a label added since or a source
    /// file pushed since must not merge (ADR 0007). Human-merge now is kept
    /// and labelled. A No-review PR that is one no longer loses
    /// orqa:no-review, so the bots review it, and waits for them. True when
    /// the merge may go on.
    pub(super) fn may_merge(&self, ticket: &str, ts: &TicketState, head: &str) -> bool {
        let pr_ref = pr_ref(&ts.pr);
        if self.is_human_merge(ticket, &ts.pr) {
            self.update(ticket, |ts| ts.human_merge = true);
            let labelled = self.add_pr_label(&ts.pr, HUMAN_MERGE_LABEL);
            let text = match labelled {
                Ok(()) => format!(
                    "{pr_ref} is human-merge now, labelled {}",
                    HUMAN_MERGE_LABEL.0
                ),
                Err(err) => format!("{pr_ref} is human-merge now, not labelled: {err}"),
            };
            self.report(ticket, &text);
            return false;
        }
        if !ts.no_review || self.changes_no_review(ticket, &ts.pr, head) {
            return true;
        }
        self.update(ticket, |ts| ts.no_review = false);
        let name = NO_REVIEW_LABEL.0;
        let argv = ["gh", "pr", "edit", &ts.pr, "--remove-label", name];
        let text = match self.cfg.tools.run(&self.cfg.repo, &argv) {
            Ok(_) => format!("{pr_ref} changes more than Markdown and skills now: {name} removed, the bots review it"),
            Err(err) => format!("{pr_ref} changes more than Markdown and skills now, {name} not removed: {err}"),
        };
        self.report(ticket, &text);
        false
    }

    /// Puts the label on the PR. gh refuses one the repo lacks, so when it
    /// fails the label is created and it is tried again: a missing label
    /// is created once, and a repo that has it needs no right to create
    /// one. --force, so creating one already there is no error of its own.
    fn add_pr_label(&self, pr: &str, (name, color, description): GhLabel) -> Result<(), RunError> {
        let (tools, repo) = (&self.cfg.tools, &self.cfg.repo);
        let add = ["gh", "pr", "edit", pr, "--add-label", name];
        if tools.run(repo, &add).is_ok() {
            return Ok(());
        }
        let create = [
            "gh",
            "label",
            "create",
            name,
            "--color",
            color,
            "--description",
            description,
            "--force",
        ];
        tools.run(repo, &create)?;
        tools.run(repo, &add).map(drop)
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
        let found = self.run_extra_review(ticket, FINAL)?;
        self.report(
            ticket,
            &format!(
                "extra review before the PR found {}",
                plural(found.found.len(), "finding")
            ),
        );
        let fixes = if extra.debate {
            let file = self
                .run_dir(ticket)
                .join(result_name(&EXTRA_REVIEW, FINAL))
                .display()
                .to_string();
            let inputs = [("Review file", file.as_str())];
            self.debate(ticket, FINAL, &inputs, found.found.len(), verdicts)?
        } else {
            not_debated_items(&found)
        };
        self.fix(ticket, FINAL, &fixes, true, "", verdicts)
    }

    /// The Extra review in `round`; run_stage fetches for it (fetch_inputs).
    fn run_extra_review(&self, ticket: &str, round: usize) -> Result<StageResult, StageError> {
        let want = ResultRequirements::default();
        self.run_read_only(ticket, &EXTRA_REVIEW, round, &[], want)
    }

    /// The Inputs an Extra review's fresh session gets from its fetch: when
    /// its skill's folder in the worktree has a fetch.sh, that runs first
    /// and the review gets its cache as the Cache Input, and, run without
    /// it, the Fetch Input. Asked only where a session starts fresh, so a
    /// resumed one, which may be reading the cache, is not fetched for,
    /// and one whose resume failed is. None for any other Stage.
    pub(super) fn fetch_inputs(
        &self,
        ticket: &str,
        st: &Stage,
        label: &str,
    ) -> Result<Vec<(&'static str, String)>, StageError> {
        if st.name != EXTRA_REVIEW.name {
            return Ok(Vec::new());
        }
        let labels = self
            .labels(ticket)
            .map_err(|err| StageError::Parked(format!("Ticket labels not read: {err}")))?;
        let Some(extra) = app::extra_review(&self.cfg.repo, &labels).map_err(StageError::Parked)?
        else {
            return Ok(Vec::new());
        };
        let script = self
            .worktree(ticket)
            .join(FILES)
            .join(&extra.skill)
            .join("fetch.sh");
        if !script.exists() {
            return Ok(Vec::new());
        }
        let cache = self.cfg.repo.join(LOCAL).join("cache").join(&extra.label);
        let mut inputs = vec![("Cache", cache.display().to_string())];
        if let Some(error) = self.run_fetch_sh(ticket, label, &script, &cache)? {
            inputs.push(("Fetch", format!("not run: {error}")));
        }
        Ok(inputs)
    }

    /// The remote's default branch as the Ticket's worktree names it,
    /// "origin/main"; origin/main when origin/HEAD is unset.
    pub(super) fn origin_head(&self, ticket: &str) -> String {
        let origin = ["git", "symbolic-ref", "--short", "refs/remotes/origin/HEAD"];
        let worktree = self.worktree(ticket);
        let head = self.cfg.tools.run(&worktree, &origin).unwrap_or_default();
        match head.trim() {
            "" => "origin/main".into(),
            h => h.into(),
        }
    }

    /// Runs an Extra review skill's fetch.sh `script` with network, before
    /// the review's pane starts: bash in the worktree, ORQA_CACHE the
    /// label's `cache` in the checkout (made first, shared by every
    /// worktree), ORQA_BASE the base branch. The Ticket reads "fetching"
    /// while it runs. Implement and Fix can write the worktree, and this
    /// runs outside their sandbox, so a fetch.sh that is not the one
    /// committed where the branch left its base is not run: a failure too.
    /// A failure is a Question with its error: retry runs it again, run
    /// without gives the error back for the Fetch Input, park parks; Away,
    /// the Ticket parks, and /continue asks again.
    fn run_fetch_sh(
        &self,
        ticket: &str,
        label: &str,
        script: &Path,
        cache: &Path,
    ) -> Result<Option<String>, StageError> {
        let (tools, worktree) = (&self.cfg.tools, self.worktree(ticket));
        let base = &self.origin_head(ticket);
        let argv = [
            "env",
            &format!("ORQA_CACHE={}", cache.display()),
            &format!("ORQA_BASE={base}"),
            "bash",
            &script.display().to_string(),
        ];
        let unchanged = || {
            let fork = ["git", "merge-base", "HEAD", base];
            let fork = tools
                .run(&worktree, &fork)
                .map_err(|err| fetch_error(&err))?;
            let rel = script.strip_prefix(&worktree).unwrap_or(script);
            let at = format!("{}:{}", fork.trim(), rel.display());
            match (
                fs::read_to_string(script),
                tools.run(&worktree, &["git", "show", &at]),
            ) {
                (Ok(text), Ok(committed)) if text == committed => Ok(()),
                _ => Err(format!(
                    "it is not the one committed on {base}, so it was not run"
                )),
            }
        };
        loop {
            self.update(ticket, |ts| ts.fetching = true);
            // ponytail: the call blocks, so /stop-work waits for fetch.sh to
            // exit; kill it on stop if fetches run long.
            let ran = local_dir(&self.cfg.repo)
                .and_then(|_| fs::create_dir_all(cache))
                .map_err(|err| err.to_string())
                .and_then(|()| unchanged())
                .and_then(|()| tools.run(&worktree, &argv).map_err(|err| fetch_error(&err)));
            self.update(ticket, |ts| ts.fetching = false);
            let Err(error) = ran else {
                return Ok(None);
            };
            let text = format!("fetch.sh for {label} failed: {error}");
            let options = ["retry", "run without it", "park"]
                .map(String::from)
                .to_vec();
            match self.ask_at_start(ticket, &text, options, |options| Ask::TicketStart {
                options,
            }) {
                Ok(0) => {}
                Ok(1) => {
                    self.report(ticket, &format!("running {label} without its fetch"));
                    return Ok(Some(error));
                }
                // /park
                Err(StageError::Parked(reason)) if reason != AWAY => {
                    return Err(StageError::Parked(text))
                }
                // Away, or /stop-work
                Err(err) => return Err(err),
                // park
                Ok(_) => return Err(StageError::Parked(text)),
            }
        }
    }

    /// A Fix given only the fix items ("none" without any); one that opens
    /// the PR also gets the Verdict files, its template and the Extra
    /// review's result files, for the PR description.
    fn fix(
        &self,
        ticket: &str,
        round: usize,
        fixes: &[String],
        open_pr: bool,
        unreviewed: &str,
        verdicts: &[String],
    ) -> Result<StageResult, StageError> {
        let items = if fixes.is_empty() {
            "none".to_string()
        } else {
            format!("\n  {}", fixes.join("\n  "))
        };
        let history = verdicts.join(", ");
        let want = ResultRequirements {
            require_pr: open_pr,
            ..Default::default()
        };
        let template = open_pr.then(|| self.pr_template(ticket)).flatten();
        let extra_files: Vec<_> = (1..=FINAL)
            .map(|round| self.run_dir(ticket).join(result_name(&EXTRA_REVIEW, round)))
            .filter(|file| file.exists())
            .map(|file| file.display().to_string())
            .collect();
        let extra_files = extra_files.join(", ");
        let mut inputs = vec![("Open PR", "no"), ("Fix items", items.as_str())];
        if open_pr {
            inputs[0].1 = "yes";
            inputs.push(("Verdict history", history.as_str()));
            if let Some(template) = &template {
                inputs.push(("PR template", template));
            }
            if !extra_files.is_empty() {
                inputs.push(("Extra review files", extra_files.as_str()));
            }
        }
        if !unreviewed.is_empty() {
            inputs.push(("Unreviewed", unreviewed));
        }
        let fix = self.run_stage(ticket, &FIX, round, &inputs, want)?;
        self.report(ticket, &format!("{} done", stage_label(&FIX, round)));
        Ok(fix)
    }

    /// The template the PR is written from, as a path in the checkout, so
    /// one not yet committed or merged still works: the Area label's
    /// pr_template, else the default, else none. A mapped file since gone
    /// falls back to the default, and RECENT says so.
    pub(super) fn pr_template(&self, ticket: &str) -> Option<String> {
        let repo = &self.cfg.repo;
        // labels or config not read stop the Stage before it starts
        let labels = self.labels(ticket).unwrap_or_default();
        let mapped = app::pr_template(repo, &labels).unwrap_or_default();
        let default = repo.join(DEFAULT_TEMPLATE);
        let default = default.exists().then(|| default.display().to_string());
        if !mapped.is_empty() {
            let file = repo.join(TEMPLATE_DIR).join(&mapped);
            if file.exists() {
                return Some(file.display().to_string());
            }
            let instead = match default {
                Some(_) => ": the default instead",
                None => ", and there is no default",
            };
            let gone = format!("PR template {TEMPLATE_DIR}/{mapped} is gone{instead}");
            self.report(ticket, &gone);
        }
        default
    }

    /// A Debate over `findings` Findings, reported as settled; its Verdict
    /// file joins the history and its fix items are returned. With no
    /// Findings it does not run: an empty Verdict is written in its place,
    /// so the Round still counts.
    fn debate(
        &self,
        ticket: &str,
        round: usize,
        inputs: &[(&str, &str)],
        findings: usize,
        verdicts: &mut Vec<String>,
    ) -> Result<Vec<String>, StageError> {
        let label = stage_label(&DEBATE, round);
        let file = self.run_dir(ticket).join(result_name(&DEBATE, round));
        if findings == 0 {
            let empty =
                "STATUS: done\n\n## Verdict\n\n## Notes\n\nNo Findings: the Debate did not run.\n";
            fs::write(&file, empty).map_err(|err| {
                StageError::Parked(format!("{label}: empty Verdict not written: {err}"))
            })?;
            self.report(ticket, &format!("{label} skipped: no findings"));
            verdicts.push(file.display().to_string());
            return Ok(Vec::new());
        }
        let verdict = self.run_read_only(
            ticket,
            &DEBATE,
            round,
            inputs,
            ResultRequirements {
                review_findings: findings,
                ..Default::default()
            },
        )?;
        self.report(
            ticket,
            &format!(
                "{label} settled: {} to fix, {} skipped",
                verdict.fixes.len(),
                verdict.skips.len()
            ),
        );
        verdicts.push(file.display().to_string());
        Ok(verdict.fixes)
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
    /// again. The last Fix's screenshots in pr/ go too, attached by then.
    /// Keeping only the evidence survives the next Stage inventing a
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
    pub(super) fn prepare_worktree(&self, ticket: &str) -> Result<(), StageError> {
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
            // the Release has no bd issue to mark
            let update = ["bd", "update", ticket, "--status", "in_progress"];
            if !self.is_release(ticket) {
                if let Err(err) = tools.run(repo, &update) {
                    self.log(ticket, &format!("not marked in_progress: {err}"));
                }
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
            (&REBASE, REBASE.name),
            (&ADDRESS_PR_COMMENTS, app::row_key(&ADDRESS_PR_COMMENTS)),
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
                for st in EDITING {
                    if !(implemented && st.name == IMPLEMENT.name) {
                        loaded.push((skill, app::row_key(st)));
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
