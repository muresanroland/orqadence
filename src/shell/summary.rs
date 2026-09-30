//! The summary of an Epic or a Ticket run: each Ticket's PR, Rounds and
//! Findings, and the Parked ones with their reasons, built fresh from bd,
//! the State and each Ticket's Run directory evidence (harness-0sx.5,
//! layout B), with each Ticket's cost and time (orchestrator/cost.rs). The
//! pager that shows it is draw/pager.rs.

use std::cell::Cell;
use std::path::Path;

use super::suffix_order;
use crate::orchestrator::app;
use crate::orchestrator::cost::{self, Cost, Logged, Span};
use crate::orchestrator::pipeline::MAX_ROUNDS;
use crate::orchestrator::result::{read_stage_result, ResultRequirements, StageResult};
use crate::orchestrator::scheduler::BdIssue;
use crate::orchestrator::stage::{plural, result_name, run_dir, DEBATE, EXTRA_REVIEW, FINAL, FIX};
use crate::orchestrator::state::{State, STATUS_MERGED, STATUS_PARKED};

/// One Epic's or Ticket run's summary, as it stood when built.
pub(crate) struct Summary {
    /// The Epic's id and title; a Ticket run has no id, and its count of
    /// Tickets for a title.
    pub(crate) epic: String,
    pub(crate) title: String,
    /// The Epic's child Tickets, in suffix order, or the Ticket run's, in
    /// the order added.
    pub(crate) tickets: Vec<Ticket>,
    /// Every Ticket's cost summed, and the run's time on the wall clock.
    pub(crate) cost: Cost,
    pub(crate) time: Option<chrono::TimeDelta>,
    /// The first body row shown; the draw keeps it inside.
    pub(crate) scroll: Cell<usize>,
    /// When it was built, for its count of the lines on RECENT since.
    pub(crate) opened: chrono::DateTime<chrono::Local>,
}

/// One Ticket of the Epic summary.
pub(crate) struct Ticket {
    /// The Ticket's id and title.
    pub(crate) id: String,
    pub(crate) title: String,
    /// The Fix result's PR url; empty before one opened.
    pub(crate) pr: String,
    /// Closed in bd, or merged by the run.
    pub(crate) merged: bool,
    /// One per Verdict.
    pub(crate) rounds: usize,
    /// The fix items a later Review re-checked.
    pub(crate) fixed: usize,
    /// Every Verdict's skip items, as shown.
    pub(crate) skipped: Vec<String>,
    /// The fix items of the cap's Verdict and of the final Fix, which no
    /// Review re-checked, on a Ticket whose PR opened: listed on the PR.
    pub(crate) left: Vec<String>,
    /// Why it is Parked.
    pub(crate) parked: Option<String>,
    /// Its sessions' tokens and API-equivalent cost, and the Apps it ran on.
    pub(crate) cost: Cost,
    /// Its time in the orchestrator log.
    pub(crate) time: Option<Span>,
}

impl Summary {
    /// The Epic's summary, or with no Epic the State's Ticket run's, from
    /// bd's issues, the State, the Run directories, the transcripts under
    /// home and the orchestrator log; an unknown Epic, or a run none of
    /// whose Tickets has run, is the error.
    pub(crate) fn build(
        repo: &Path,
        home: &Path,
        issues: &[BdIssue],
        state: &State,
        epic: &str,
    ) -> Result<Summary, String> {
        let (title, children) = match epic {
            "" => {
                let queued = state.queue.iter();
                let children: Vec<&BdIssue> = queued
                    .filter_map(|id| issues.iter().find(|i| i.id == *id))
                    .collect();
                (plural(children.len(), "Ticket"), children)
            }
            _ => {
                let title = issues
                    .iter()
                    .find(|i| i.id == epic && i.issue_type == "epic")
                    .ok_or(format!("no Epic {epic} in bd"))?
                    .title
                    .clone();
                let mut children: Vec<&BdIssue> = issues
                    .iter()
                    .filter(|i| i.parent == epic && i.issue_type != "epic")
                    .collect();
                children.sort_by_key(|i| suffix_order(&i.id));
                (title, children)
            }
        };
        let ran = children
            .iter()
            .any(|t| run_dir(repo, &t.id).exists() || state.tickets.contains_key(&t.id));
        if !ran {
            let run = if epic.is_empty() {
                "the Ticket run"
            } else {
                epic
            };
            return Err(format!(
                "no evidence for {run}: none of its Tickets has run"
            ));
        }
        let ids: Vec<&str> = children.iter().map(|t| t.id.as_str()).collect();
        let (mut costs, mut logged) = (cost::costs(home, repo, &ids), cost::logged(repo));
        // The Debate sides' Apps: headless, they are in no State session.
        let sides: Vec<String> = app::read(repo).map_or(Vec::new(), |(_, doc)| {
            ["side_a", "side_b"]
                .iter()
                .filter_map(|side| app::field(&doc, side, "app").ok())
                .collect()
        });
        let tickets: Vec<Ticket> = children
            .into_iter()
            .map(|t| {
                let spent = (costs.remove(&t.id), logged.remove(&t.id));
                ticket(repo, state, t, spent, &sides)
            })
            .collect();
        let mut total = Cost::default();
        tickets.iter().for_each(|t| total.add(&t.cost));
        Ok(Summary {
            epic: epic.to_string(),
            title,
            time: cost::wall_clock(tickets.iter().filter_map(|t| t.time.as_ref())),
            cost: total,
            tickets,
            scroll: Cell::new(0),
            opened: chrono::Local::now(),
        })
    }
}

/// One Ticket from its Run directory: a Round per verdict-N.md, the PR from
/// the final Fix's result or the last Round's, merged and parked from bd and
/// the State. Its Apps are its transcripts', its log's, its State sessions'
/// and, once it has had a Debate, the sides'.
fn ticket(
    repo: &Path,
    state: &State,
    t: &BdIssue,
    (cost, logged): (Option<Cost>, Option<Logged>),
    sides: &[String],
) -> Ticket {
    let dir = run_dir(repo, &t.id);
    let read = |name: String| read_stage_result(&dir.join(name), ResultRequirements::default()).0;
    let verdicts: Vec<StageResult> = (1..=MAX_ROUNDS)
        .take_while(|n| dir.join(result_name(&DEBATE, *n)).exists())
        .map(|n| read(result_name(&DEBATE, n)))
        .collect();
    let rounds = verdicts.len();
    // An Extra review before the PR, after the last Round: its Verdict, or
    // with its Debate off its Findings as the final Fix's items.
    let before_pr = if dir.join(result_name(&DEBATE, FINAL)).exists() {
        read(result_name(&DEBATE, FINAL))
    } else {
        StageResult {
            fixes: read(result_name(&EXTRA_REVIEW, FINAL)).found,
            ..Default::default()
        }
    };
    let pr = [FINAL, rounds]
        .map(|n| read(result_name(&FIX, n)).pr)
        .into_iter()
        .find(|pr| !pr.is_empty())
        .unwrap_or_default();
    // Once the PR opened: the cap's fix items and the final Fix's, which no
    // Review re-checked.
    let left: Vec<String> = match pr.is_empty() {
        true => Vec::new(),
        false => verdicts
            .last()
            .filter(|_| rounds == MAX_ROUNDS)
            .into_iter()
            .chain([&before_pr])
            .flat_map(|v| v.fixes.clone())
            .collect(),
    };
    let ts = state.tickets.get(&t.id);
    let (mut cost, logged) = (cost.unwrap_or_default(), logged.unwrap_or_default());
    cost.apps.extend(logged.apps);
    cost.apps.extend(
        ts.iter()
            .flat_map(|ts| ts.sessions.values().map(|s| s.app.clone())),
    );
    if rounds > 0 {
        cost.apps.extend(sides.iter().cloned());
    }
    Ticket {
        id: t.id.clone(),
        title: t.title.clone(),
        pr,
        merged: t.status == "closed" || ts.is_some_and(|ts| ts.status == STATUS_MERGED),
        rounds,
        // all but the last Verdict: no Review re-checked its fix items
        fixed: verdicts.iter().rev().skip(1).map(|v| v.fixes.len()).sum(),
        skipped: verdicts
            .iter()
            .chain([&before_pr])
            .flat_map(|v| &v.skips)
            .map(|l| finding(l))
            .collect(),
        left: left.iter().map(|l| finding(l)).collect(),
        parked: ts
            .filter(|ts| ts.status == STATUS_PARKED)
            .map(|ts| ts.reason.clone()),
        cost,
        time: logged.span,
    }
}

/// A Verdict item as the summary shows it: its severity, place and problem,
/// without the mark, the reason or how it was settled; a Finding not
/// debated is as the Review wrote it.
fn finding(line: &str) -> String {
    let bare = line.strip_prefix("- ").unwrap_or(line);
    let item = line.split_once("] ").map_or(bare, |(_, rest)| rest);
    item.split_once(" | reason:")
        .map_or(item, |(f, _)| f)
        .to_string()
}
