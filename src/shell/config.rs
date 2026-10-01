//! /config: the App, model and effort of each row of .orqadence/config.json,
//! picked in a modal docked beside the live Shell (layout C of
//! harness-0sx.9, drawn in draw/config.rs) and saved at once. A Stage reads
//! its row as it starts, so the Stages that start after a change use it and
//! running ones keep theirs. A named model is probed with a one-line prompt
//! first, off the screen thread; it saves only if the App answers. Also each
//! job's Delegate skill, the skills Orqadence installed (harness-0sx.7),
//! cloned off the screen thread too, TypeSafe on or off with its key and
//! the Judgments' floors, the Tickets and PR sessions a run takes at once,
//! Rebase and Address PR comments' switches, countdown and cap, the
//! Release's switch, On call's Moshi token, minutes and test push, and the
//! Ticket labels: each entry of config.json's labels, area or modifier,
//! with its skills and guidance.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use crossterm::event::KeyCode;
use ratatui::style::Color;
use serde_json::{json, Value};

use super::brand::{CYAN, GREEN, MUTED, ORANGE, RED};
use super::{NoticeKind, Screen, NOTICE_WINDOW};
use crate::on_call::{self, OnCall, DEFAULT_MINUTES};
use crate::orchestrator::app::{
    self, app, App, Check, Count, Floor, Label, Model, Row, Switch, ADDRESS_PR_COMMENTS_AUTO,
    ADDRESS_PR_COMMENTS_COUNTDOWN, ADDRESS_PR_COMMENTS_RUNS, AGENT_MERGE, APPS, BOT_WAIT,
    IF_LIMITED, MAX_PR_SESSIONS, MAX_TICKETS, REBASE_AUTO, RELEASE, RELEASE_ON, REVIEW_BOTS,
    REVIEW_BOTS_KEY,
};
use crate::orchestrator::judgment::{PLAN_FLOOR, WAKE_FLOOR};
use crate::orchestrator::stage::plural;
use crate::setup;
use crate::skills::manifest::{self, job_row, parse_source, Added, Manifest, JOBS, NONE};
use crate::tools::{RunError, Tools};

/// What a save line adds (ADR 0006): the file is the working copy's, for
/// you to commit. A setting or a pick applies at once from it; a skill
/// change reaches a Ticket through its base.
const CONFIG: &str = ", saved uncommitted in .orqadence/config.json";
const PICK: &str = ", saved uncommitted in .orqadence/skills.json";
const SKILL: &str = "; saved uncommitted: Tickets take the change once it is merged";

/// One row of config.json: its key, its name, the lead of its settings'
/// labels on a section's page ("" for the section's own row), the section
/// it sits in, and what it runs.
pub(crate) struct ConfigRow {
    pub(crate) key: &'static str,
    pub(crate) name: &'static str,
    pub(crate) lead: &'static str,
    pub(crate) section: usize,
    pub(crate) note: &'static str,
}

pub(crate) const ROWS: [ConfigRow; 10] = [
    ConfigRow {
        key: "implement",
        name: "Implement",
        lead: "",
        section: 0,
        note: "Plans, implements test-first, reviews itself and commits in the Ticket's worktree.",
    },
    ConfigRow {
        key: "review",
        name: "Review",
        lead: "",
        section: 1,
        note: "Reviews the diff into Findings.",
    },
    ConfigRow {
        key: IF_LIMITED,
        name: "Review if limited",
        lead: "if limited:",
        section: 1,
        note: "Runs the Review when the Review's App is Limited and you answer to review with it; none leaves wait or open the PR unreviewed.",
    },
    ConfigRow {
        key: "moderator",
        name: "Moderator",
        lead: "Moderator",
        section: 2,
        note: "The Moderator's pane: runs the Debate and settles each Finding.",
    },
    ConfigRow {
        key: "side_a",
        name: "Debate side A",
        lead: "side A",
        section: 2,
        note: "Argues each Finding, headless; also runs the ponytail audit.",
    },
    ConfigRow {
        key: "side_b",
        name: "Debate side B",
        lead: "side B",
        section: 2,
        note: "Argues each Finding, headless, on a model from another family than side A.",
    },
    ConfigRow {
        key: "fix",
        name: "Fix",
        lead: "",
        section: 3,
        note: "Fixes the Findings to fix, then opens the PR.",
    },
    ConfigRow {
        key: "rebase",
        name: "Rebase",
        lead: "",
        section: 4,
        note: "Rebases a PR that conflicts with main, keeping both sides' intent or asking you.",
    },
    ConfigRow {
        key: "address_pr_comments",
        name: "Address PR comments",
        lead: "",
        section: 5,
        note: "Fixes a PR's comments and failing checks, and pushes to the same PR.",
    },
    ConfigRow {
        key: RELEASE,
        name: "Release",
        lead: "",
        section: 6,
        note: "Raises the Target repo's version, adds a changelog entry where the repo keeps one, and opens the version PR.",
    },
];

/// The Pipeline's sections: title, short name on the left, description.
pub(crate) const SECTIONS: [(&str, &str, &str); 7] = [
    (
        "Plan + Implement",
        "Plan+Impl",
        "In the Ticket's worktree: plans in plan mode, then implements test-first, reviews itself and commits.",
    ),
    (
        "Review",
        "Review",
        "Reviews the diff into Findings, respecting earlier Verdicts.",
    ),
    (
        "Debate",
        "Debate",
        "The Moderator puts each disputed Finding to side A and side B, two models from different families; TypeSafe scores what they still dispute.",
    ),
    ("Fix", "Fix", "Fixes the Findings to fix, then opens the PR."),
    (
        "Rebase",
        "Rebase",
        "Rebases a PR that conflicts with main onto it, keeping both sides' intent or asking you.",
    ),
    (
        "Address PR comments",
        "Comments",
        "Fixes a PR's comments and failing checks, and pushes to the same PR.",
    ),
    (
        "Release",
        "Release",
        "Turned on, a run whose Epic, or any Ticket of a Ticket run, carries orqa:release ends in a Release once every Ticket is merged: an Epic run raises the minor version, a Ticket run the patch. Off, the label is ignored.",
    ),
];

/// The Rebase, Address PR comments and Release sections, whose pages have a
/// switch.
pub(crate) const REBASE_PAGE: usize = 4;
pub(crate) const ADDRESS_PR_COMMENTS_PAGE: usize = 5;
pub(crate) const RELEASE_PAGE: usize = 6;
/// The Apps page's place on the left, after the Pipeline's sections, and
/// the Skills, Labels, TypeSafe, Run and On call pages' after it.
pub(crate) const APPS_PAGE: usize = SECTIONS.len();
pub(crate) const SKILLS_PAGE: usize = APPS_PAGE + 1;
pub(crate) const LABELS_PAGE: usize = APPS_PAGE + 2;
pub(crate) const TYPESAFE_PAGE: usize = APPS_PAGE + 3;
pub(crate) const RUN_PAGE: usize = APPS_PAGE + 4;
pub(crate) const ON_CALL_PAGE: usize = APPS_PAGE + 5;
/// The row the Extra review runs on: the Review's.
pub(crate) const REVIEW_ROW: usize = 1;
/// The Skills page's rows before its skills: the location and your
/// personal skills' switch.
pub(crate) const SKILL_ROWS: usize = 2;

/// The floors the TypeSafe page shows under the key, in its order.
pub(crate) const FLOORS: [&Floor; 2] = [&WAKE_FLOOR, &PLAN_FLOOR];

/// A floor as the pages name it: "wake floor".
pub(crate) fn floor_name(floor: &Floor) -> String {
    floor.key.replace('_', " ")
}

/// A whole number of config.json a page keeps: its Count, the page it is
/// on, its label there and the foot's note on it.
#[derive(Debug, PartialEq)]
pub(crate) struct Number {
    pub(crate) count: &'static Count,
    pub(crate) page: usize,
    pub(crate) name: &'static str,
    note: &'static str,
}

/// The whole numbers the pages keep, in their order on a page; static, so a
/// Check can borrow a key.
pub(crate) static NUMBERS: [Number; 5] = [
    Number {
        count: &MAX_TICKETS,
        page: RUN_PAGE,
        name: "tickets at once",
        note: "Tickets in the Pipeline at once, in an Epic run or a Ticket run.",
    },
    Number {
        count: &MAX_PR_SESSIONS,
        page: RUN_PAGE,
        name: "PR sessions at once",
        note: "Rebase and Address PR comments sessions at once, together, apart from max_tickets.",
    },
    Number {
        count: &ADDRESS_PR_COMMENTS_COUNTDOWN,
        page: ADDRESS_PR_COMMENTS_PAGE,
        name: "countdown minutes",
        note: "Minutes the approval modal counts down before it approves the checked items; 0 is no countdown: the modal waits.",
    },
    Number {
        count: &ADDRESS_PR_COMMENTS_RUNS,
        page: ADDRESS_PR_COMMENTS_PAGE,
        name: "runs per PR",
        note: "Address PR comments runs per PR; past it new items only get a line, and /address-pr-comments still works by hand.",
    },
    Number {
        count: &BOT_WAIT,
        page: ADDRESS_PR_COMMENTS_PAGE,
        name: "bot wait minutes",
        note: "Minutes Agent merge waits for a ticked review bot's review; past it the merge is a Question.",
    },
];

/// Each Stage page's switch: the section and the Switch.
const SWITCHES: [(usize, &Switch); 3] = [
    (REBASE_PAGE, &REBASE_AUTO),
    (ADDRESS_PR_COMMENTS_PAGE, &ADDRESS_PR_COMMENTS_AUTO),
    (RELEASE_PAGE, &RELEASE_ON),
];

/// What turning TypeSafe off changes, asked first.
const TYPESAFE_OFF: &str = "Turn TypeSafe off? Every Wake and Plan becomes a Question; disputed Findings are skipped and listed in the PR.";

/// A setting of a row.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Field {
    App,
    Model,
    Effort,
    /// Implement's plan model, other than its model on a split.
    Plan,
    /// Plan + Implement's 'Same model for plan and implementation'.
    Same,
    /// A job's Delegate skill, of JOBS, kept in the Skill manifest.
    Job(usize),
    /// The open label's skills, on the Labels page.
    // ponytail: this and Template and ExtraSkill ride Pick with row 0, which
    // they never read; a list of its own when a fourth pick outside the rows
    // comes.
    Skills,
    /// The open label's PR template, on the Labels page.
    Template,
    /// The open label's Extra review skill.
    ExtraSkill,
    /// The page's switch, on the Rebase, Address PR comments and Release
    /// pages.
    Switch(&'static Switch),
    /// A whole number on a Stage's page.
    Number(&'static Number),
    /// A review bot's checkbox, of REVIEW_BOTS, under Agent merge.
    Bot(usize),
}

/// Where a pick's row lives: config.json's (the Stage pages), the open
/// label's override of it, or the open label's Extra review, which runs on
/// the Review's row.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Scope {
    Repo,
    Label,
    Extra,
}

/// A row of the open label's page, in its order; the Extra review's for an
/// Area label only.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum LabelItem {
    Kind,
    Skills,
    Guidance,
    Template,
    ExtraSkill,
    Position,
    Debate,
    /// The Extra review's App, model or effort.
    Extra(Field),
    /// A Stage row's App, model or effort this label overrides.
    Row(usize, Field),
}

/// The fields of a row a label overrides.
const FIELDS: [Field; 3] = [Field::App, Field::Model, Field::Effort];

/// When an Extra review runs, as its position in config.json says.
pub(crate) fn position_name(position: &str) -> &'static str {
    match position {
        "first" => "first Round only",
        "before_pr" => "before the PR",
        _ => "every Round",
    }
}

impl Field {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Field::Plan => "plan model",
            Field::Same => "same model",
            Field::Switch(switch) => switch.question.trim_end_matches('?'),
            Field::Number(n) => n.name,
            Field::Bot(b) => REVIEW_BOTS[b],
            field => field.key(),
        }
    }

    /// Its name in config.json.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Field::App => "app",
            Field::Model => "model",
            Field::Effort => "effort",
            Field::Plan => "plan_model",
            Field::Same => "same",
            Field::Job(j) => JOBS[j].0,
            Field::Skills => "skills",
            Field::Template => "pr_template",
            Field::ExtraSkill => "skill",
            Field::Switch(switch) => switch.key,
            Field::Number(n) => n.count.key,
            Field::Bot(_) => REVIEW_BOTS_KEY,
        }
    }
}

/// A job as a Stage's page names it.
pub(crate) fn job_name(j: usize) -> String {
    match JOBS[j].0 {
        "audit" => "over-engineering audit".to_string(),
        "test-first" => "test-first".to_string(),
        "pr-comments" => "PR comments".to_string(),
        job => job.replace('-', " "),
    }
}

/// The row whose App runs a job's line.
pub(crate) fn job_row_of(j: usize) -> usize {
    let key = job_row(JOBS[j].0);
    ROWS.iter().position(|r| r.key == key).unwrap()
}

/// A job with its section: "Plan + Implement test-first".
pub(crate) fn job_said(j: usize) -> String {
    format!(
        "{} {}",
        SECTIONS[ROWS[job_row_of(j)].section].0,
        job_name(j)
    )
}

/// A clone URL as the pages show it: owner/repo on GitHub.
pub(crate) fn short(repo: &str) -> &str {
    repo.strip_prefix("https://github.com/").unwrap_or(repo)
}

/// A commit as the pages show it.
pub(crate) fn short_commit(commit: &str) -> &str {
    commit.get(..7).unwrap_or(commit)
}

/// Every skill you have, each with its folder as the pages show it: from the
/// repo's root, or ~ for home.
fn found(repo: &Path, home: &Path, tools: &dyn Tools) -> Vec<(String, PathBuf)> {
    manifest::list(repo, home, tools, manifest::personal(repo))
        .into_iter()
        .map(|(name, path)| {
            let dir = path.parent().unwrap_or(&path);
            let shown = match (dir.strip_prefix(repo), dir.strip_prefix(home)) {
                (Ok(rel), _) => rel.to_path_buf(),
                (_, Ok(rel)) if !home.as_os_str().is_empty() => Path::new("~").join(rel),
                _ => dir.to_path_buf(),
            };
            (name, shown)
        })
        .collect()
}

/// An open pick list, which replaces the section's page.
pub(crate) struct Pick {
    pub(crate) scope: Scope,
    pub(crate) row: usize,
    pub(crate) field: Field,
    /// A model list for a new App, picked just before: the pair saves together.
    pub(crate) app: Option<&'static App>,
    pub(crate) cursor: usize,
    pub(crate) filter: String,
}

/// What a pick list's entry picks.
#[derive(Clone)]
pub(crate) enum Picked {
    App(&'static App),
    /// A model, an effort or a skill, as the pick's field says.
    Value(String),
    /// 'type an id…'
    Typed,
    /// A suggested skill not installed: its name and source, installed first.
    Install(&'static str, &'static str),
}

/// A line of a pick list; one that picks nothing is a heading.
pub(crate) struct Entry {
    pub(crate) name: String,
    pub(crate) detail: String,
    pub(crate) current: bool,
    /// The rule a model picked would break: '✗ side A's family'; how the
    /// job's App has a skill: 'installed', 'yours'.
    pub(crate) mark: Option<(&'static str, Color)>,
    /// Greyed: an App not installed.
    pub(crate) dim: bool,
    pub(crate) picks: Option<Picked>,
}

/// A named model being tried before its change saves.
pub(crate) struct Probe {
    result: Receiver<Result<String, RunError>>,
    scope: Scope,
    row: usize,
    fields: Vec<(Field, String)>,
    pub(crate) app: &'static str,
    pub(crate) model: String,
}

/// What the foot's line is typing.
pub(crate) enum Typing {
    /// A model id, for the model list it came from.
    Model(Pick),
    /// A skill's source, on the Skills page.
    Source,
    /// The TypeSafe key, shown as dots.
    Key,
    /// A floor, on the TypeSafe page.
    Floor(&'static Floor),
    /// A whole number, on the Run page or a Stage's.
    Number(&'static Number),
    /// The Moshi token, shown as dots, on the On call page.
    Token,
    /// On call's minutes.
    Minutes,
    /// A new label's name, on the Labels page.
    LabelName,
    /// A new PR template's section heading.
    Heading,
    /// A label's new name, the old one held.
    Rename(String),
    /// The open label's guidance line.
    Guidance,
}

/// The skills of a source that holds several: each name, whether it is
/// installed from there already (ticked for good), and whether it is ticked.
pub(crate) struct Listing {
    pub(crate) source: String,
    pub(crate) names: Vec<(String, bool, bool)>,
    pub(crate) cursor: usize,
}

/// What a yes to the foot's question does.
pub(crate) enum Confirm {
    Remove(String),
    TypeSafeOff,
    DeleteLabel(String),
}

/// A clone or an update going on its own thread, said in the foot.
pub(crate) struct Busy {
    pub(crate) text: String,
    done: Receiver<(Done, Vec<(String, PathBuf)>)>,
}

/// What a clone came back with.
enum Done {
    /// A pasted source, and what add made of it.
    Added(String, Result<Added, String>),
    /// The checklist's ticked skills, each with what add made of it.
    Ticked(Vec<(String, Result<Added, String>)>),
    /// A job's suggestion, installed for the job to pick.
    ForJob(usize, Result<Added, String>),
    /// One skill updated, or every one (None): those that failed, with why.
    Updated(Option<String>, Result<Vec<(String, String)>, String>),
    /// A test push: sent, or why not.
    Pushed(Result<(), String>),
}

/// /config, open.
pub(crate) struct Settings {
    pub(crate) doc: Value,
    /// Each App's models, as APPS orders them, read when /config opened.
    models: Vec<Result<Vec<Model>, String>>,
    /// Each App, as APPS orders them, on PATH: the first line its
    /// --version prints; None when not on PATH.
    pub(crate) installed: Vec<Option<String>>,
    /// The Skill manifest, read again after each change to it.
    pub(crate) manifest: Manifest,
    /// Every skill you have, as found() gives them.
    pub(crate) found: Vec<(String, PathBuf)>,
    /// Whether your personal skills are on (manifest::personal).
    pub(crate) personal: bool,
    /// The section on the left (APPS_PAGE past them), and whether the
    /// cursor is on its page.
    pub(crate) section: usize,
    pub(crate) open: bool,
    /// The setting under the cursor on the page, of Settings::items.
    pub(crate) setting: usize,
    pub(crate) pick: Option<Pick>,
    /// The foot's line being typed.
    pub(crate) typing: Option<(Typing, String)>,
    pub(crate) probe: Option<Probe>,
    /// Each App and model a probe answered while this /config is open: not
    /// probed again till it opens anew.
    probed: Vec<(&'static str, String)>,
    pub(crate) busy: Option<Busy>,
    /// A source's skills to tick, which replaces the Skills page.
    pub(crate) listing: Option<Listing>,
    /// The foot's yes/no question.
    pub(crate) confirm: Option<(String, Confirm)>,
    /// The label whose own page is open, on the Labels page.
    pub(crate) label: Option<String>,
    /// The files of .github/PULL_REQUEST_TEMPLATE, read when /config opened
    /// and when the PR template list opens.
    pub(crate) templates: Vec<String>,
    /// The foot's line in place of the row's note: saved, refused, failed.
    pub(crate) note: Option<(String, Color)>,
    /// When this /config last saved.
    pub(crate) saved: Option<String>,
}

/// A row's setting in doc, the default filled in; a field not a string
/// shows why.
fn value(doc: &Value, row: usize, field: Field) -> String {
    app::field(doc, ROWS[row].key, field.key()).unwrap_or_else(|err| err)
}

/// Implement's plan model in doc on a split: one other than its model, not
/// default.
fn split(doc: &Value) -> Option<String> {
    let plan = value(doc, 0, Field::Plan);
    (plan != "default" && plan != value(doc, 0, Field::Model)).then_some(plan)
}

/// A row in doc as RECENT's started line names it: "codex gpt-6-sol/high",
/// on a split "claude claude-fable-5-1→claude-opus-5-5", or "none" for a
/// fallback that is not set.
fn said(doc: &Value, row: usize) -> String {
    let model = value(doc, row, Field::Model);
    match app(&value(doc, row, Field::App)) {
        _ if model == "none" => model,
        None => value(doc, row, Field::App),
        Some(app) => Row {
            app,
            model,
            effort: value(doc, row, Field::Effort),
            plan_model: split(doc).filter(|_| row == 0),
        }
        .said(),
    }
}

/// The family of model on app as /config shows it.
pub(crate) fn family_label(app: &App, model: &str) -> &'static str {
    app::family_of(app, model).unwrap_or("family unknown")
}

/// The note on an App not on PATH: where to get it.
fn not_on_path(app: &App) -> String {
    format!("{} is not on PATH: get it at {}", app.bin, app.home)
}

/// The Run page's whole numbers.
pub(crate) fn run_numbers() -> impl Iterator<Item = &'static Number> {
    NUMBERS.iter().filter(|n| n.page == RUN_PAGE)
}

/// The foot's note on a whole number.
pub(crate) fn number_note(n: &Number) -> String {
    format!(
        "{} Enter types {}, or nothing for the default {}, saved at once, uncommitted: the live run reads it next.",
        n.note,
        n.count.rule(),
        n.count.default
    )
}

/// The rows of a section.
fn rows_of(section: usize) -> impl Iterator<Item = usize> {
    (0..ROWS.len()).filter(move |&r| ROWS[r].section == section)
}

/// The section a check shows on: its last row's; a whole number's, its
/// page; the review bots', Address PR comments'; a floor's, the TypeSafe
/// page.
pub(crate) fn section_of(check: &Check) -> usize {
    let key = check.rows[check.rows.len() - 1];
    match (
        ROWS.iter().find(|r| r.key == key),
        NUMBERS.iter().find(|n| n.count.key == key),
    ) {
        (Some(r), _) => r.section,
        (_, Some(n)) => n.page,
        _ if key == REVIEW_BOTS_KEY => ADDRESS_PR_COMMENTS_PAGE,
        _ => TYPESAFE_PAGE,
    }
}

/// Each once, in the order first met.
pub(crate) fn distinct(all: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for one in all {
        if !out.contains(&one) {
            out.push(one);
        }
    }
    out
}

impl Settings {
    /// A setting as it is: a job's the Skill manifest's pick.
    pub(crate) fn value(&self, row: usize, field: Field) -> String {
        match field {
            Field::Job(j) => self.manifest.pick(JOBS[j].0).to_string(),
            _ => value(&self.doc, row, field),
        }
    }

    /// Whether TypeSafe judges: not off in config.json, and with a key.
    pub(crate) fn typesafe(&self, key: &str) -> bool {
        !key.is_empty() && self.doc["typesafe"] != false
    }

    /// The skills installed but the Shipped ones.
    pub(crate) fn skills(&self) -> usize {
        self.manifest.skills.values().filter(|s| !s.shipped).count()
    }

    /// The jobs that pick a skill, each as job_said names it.
    pub(crate) fn jobs_using(&self, name: &str) -> Vec<String> {
        (0..JOBS.len())
            .filter(|&j| self.manifest.pick(JOBS[j].0) == name)
            .map(job_said)
            .collect()
    }

    /// The skills app loads, each name once with its folder, the Shipped ones
    /// left out.
    fn seen(&self, app: &App) -> Vec<(&str, &Path)> {
        let mut out: Vec<(&str, &Path)> = Vec::new();
        for (name, dir) in &self.found {
            let shipped = self.manifest.skills.get(name).is_some_and(|s| s.shipped);
            if app.loads(name, dir) && !shipped && !out.iter().any(|(n, _)| n == name) {
                out.push((name, dir));
            }
        }
        out
    }

    /// How app has a skill: built into it, installed by Orqadence (Shipped
    /// or fetched), or yours; with where from. None when it lacks it.
    pub(crate) fn have(&self, app: &App, name: &str) -> Option<(&'static str, Color, String)> {
        if app.built_in.contains(&name) {
            return Some(("built in", CYAN, format!("built into {}", app.name)));
        }
        let (_, dir) = (self.found.iter()).find(|(n, dir)| n == name && app.loads(n, dir))?;
        Some(
            match (self.manifest.skills.get(name), name.split_once(':')) {
                (Some(skill), _) if skill.shipped => ("installed", GREEN, "shipped".to_string()),
                (Some(skill), _) => ("installed", GREEN, short(&skill.repo).to_string()),
                (None, Some((plugin, _))) => ("yours", CYAN, format!("plugin {plugin}")),
                (None, None) => ("yours", CYAN, dir.display().to_string()),
            },
        )
    }

    /// The App a row runs on, None for a name no App has.
    pub(crate) fn app(&self, row: usize) -> Option<&'static App> {
        app(&self.value(row, Field::App))
    }

    /// Whether app was on PATH when /config opened.
    pub(crate) fn is_installed(&self, app: &App) -> bool {
        APPS.iter()
            .zip(&self.installed)
            .any(|(a, on)| on.is_some() && a.name == app.name)
    }

    /// The Apps line on the left: "1 of 2 installed".
    pub(crate) fn apps_summary(&self) -> String {
        let on = self.installed.iter().filter(|on| on.is_some()).count();
        format!("{on} of {} installed", APPS.len())
    }

    /// The foot's note on the Apps page's App i.
    pub(crate) fn app_note(&self, i: usize) -> String {
        match self.installed[i] {
            Some(_) => format!("{} is on PATH: a Stage can run on it.", APPS[i].name),
            None => not_on_path(&APPS[i]),
        }
    }

    /// Implement's plan model on a split.
    pub(crate) fn split(&self) -> Option<String> {
        split(&self.doc)
    }

    /// The rules on the rows as they are, each floor that is not a number
    /// from 0 to 1, each whole number that breaks its rule, and review bots
    /// that cannot be read; those of a section when given. None is a rule a
    /// run refuses to start on: a floor's Judgments ask instead, and a whole
    /// number's default is taken.
    pub(crate) fn checks(&self, section: Option<usize>) -> Vec<Check> {
        let mut checks = app::checks(&self.doc);
        for floor in FLOORS {
            if let Err(err) = app::floor_in(&self.doc, floor) {
                checks.push(Check {
                    rows: std::slice::from_ref(&floor.key),
                    label: None,
                    holds: false,
                    text: format!("{err}: its Judgments are not acted on"),
                });
            }
        }
        for n in &NUMBERS {
            if let Err(err) = app::count_in(&self.doc, n.count) {
                checks.push(Check {
                    rows: std::slice::from_ref(&n.count.key),
                    label: None,
                    holds: false,
                    text: format!("{err}: its default {} is taken", n.count.default),
                });
            }
        }
        if let Err(err) = app::review_bots_in(&self.doc) {
            checks.push(Check {
                rows: &[REVIEW_BOTS_KEY],
                label: None,
                holds: false,
                text: format!("{err}: tick the repo's bots again"),
            });
        }
        checks.retain(|c| section.is_none_or(|s| section_of(c) == s));
        checks
    }

    /// A section's line on the left: its row's App and model; the Debate's
    /// Apps.
    pub(crate) fn summary(&self, section: usize) -> String {
        if section == 2 {
            return distinct(rows_of(section).map(|r| self.value(r, Field::App))).join("+");
        }
        let row = rows_of(section).next().unwrap();
        if let Some(plan) = self.split().filter(|_| section == 0) {
            let model = self.value(row, Field::Model);
            return format!("{} {plan}→{model}", self.value(row, Field::App));
        }
        match self.value(row, Field::Model).as_str() {
            "default" => self.value(row, Field::App),
            model => format!("{} {model}", self.value(row, Field::App)),
        }
    }

    /// The open section's settings, row by row; Plan + Implement's with
    /// its toggle, and the plan model on a split; then its switch and whole
    /// numbers, Address PR comments' with Agent merge's switch, review bots
    /// and bot_wait last, and its jobs.
    pub(crate) fn items(&self) -> Vec<(usize, Field)> {
        let mut items: Vec<(usize, Field)> = if self.section == 0 {
            let plan = self.split().map(|_| (0, Field::Plan));
            let head = [(0, Field::App), (0, Field::Same)].into_iter().chain(plan);
            head.chain([(0, Field::Model), (0, Field::Effort)])
                .collect()
        } else {
            rows_of(self.section)
                .flat_map(|r| [Field::App, Field::Model, Field::Effort].map(|f| (r, f)))
                .collect()
        };
        if let Some(&(row, _)) = items.first() {
            let switch = SWITCHES.iter().filter(|(s, _)| *s == self.section);
            items.extend(switch.map(|&(_, switch)| (row, Field::Switch(switch))));
            let (bot_wait, numbers): (Vec<_>, Vec<_>) = NUMBERS
                .iter()
                .filter(|n| n.page == self.section)
                .partition(|n| *n.count == BOT_WAIT);
            items.extend(numbers.into_iter().map(|n| (row, Field::Number(n))));
            if self.section == ADDRESS_PR_COMMENTS_PAGE {
                items.push((row, Field::Switch(&AGENT_MERGE)));
                items.extend((0..REVIEW_BOTS.len()).map(|b| (row, Field::Bot(b))));
            }
            items.extend(bot_wait.into_iter().map(|n| (row, Field::Number(n))));
        }
        let jobs = (0..JOBS.len()).map(|j| (job_row_of(j), Field::Job(j)));
        items.extend(jobs.filter(|&(r, _)| ROWS[r].section == self.section));
        items
    }

    /// The doc a scope's rows are read from: config.json's, or as a Ticket
    /// with the open label runs them.
    fn view(&self, scope: Scope) -> Value {
        let name = self.label.as_deref().unwrap_or_default();
        match scope {
            Scope::Repo => self.doc.clone(),
            Scope::Label => app::with_label(&self.doc, name),
            Scope::Extra => app::with_extra(&self.doc, name),
        }
    }

    /// A pick's row setting as its scope runs it.
    fn row_value(&self, pick: &Pick, field: Field) -> String {
        value(&self.view(pick.scope), pick.row, field)
    }

    /// What the open label sets on a row field, or its Extra review's: empty
    /// falls through. The repo row's own value for Repo scope.
    pub(crate) fn own(&self, scope: Scope, row: usize, field: Field) -> String {
        if scope == Scope::Repo {
            return self.value(row, field);
        }
        let label = self.label.as_deref().map(|name| self.label_of(name));
        let Some(Ok(label)) = label else {
            return String::new();
        };
        let extra = label.extra_review;
        match (scope, field) {
            (Scope::Extra, Field::App) => extra.app,
            (Scope::Extra, Field::Model) => extra.model,
            (Scope::Extra, _) => extra.effort,
            _ => label
                .rows
                .get(ROWS[row].key)
                .and_then(|fields| fields.get(field.key()))
                .cloned()
                .unwrap_or_default(),
        }
    }

    /// What an empty field falls through to: the repo's row, the Review's
    /// for the Extra review.
    pub(crate) fn fallback(&self, scope: Scope, row: usize, field: Field) -> String {
        let name = self.label.as_deref().unwrap_or_default();
        match scope {
            Scope::Extra => value(&app::with_label(&self.doc, name), row, field),
            _ => value(&self.doc, row, field),
        }
    }

    /// The App a pick's models and efforts are for.
    pub(crate) fn pick_app(&self, pick: &Pick) -> Option<&'static App> {
        pick.app.or_else(|| app(&self.row_value(pick, Field::App)))
    }

    /// What app listed when /config opened.
    fn catalog(&self, app: &App) -> &Result<Vec<Model>, String> {
        &self.models[APPS.iter().position(|a| a.name == app.name).unwrap()]
    }

    fn listed(&self, app: &App) -> &[Model] {
        self.catalog(app).as_deref().unwrap_or_default()
    }

    /// The effort levels of model on app: its own when listed, else every
    /// listed model's.
    fn efforts(&self, app: &App, model: &str) -> Vec<String> {
        let listed = self.listed(app);
        match listed.iter().find(|(id, _)| id == model) {
            Some((_, efforts)) => efforts.clone(),
            None => distinct(listed.iter().flat_map(|(_, efforts)| efforts.clone())),
        }
    }

    /// A pick list's lines, filtered: a heading only unfiltered, 'type an
    /// id…' always.
    pub(crate) fn entries(&self, pick: &Pick) -> Vec<Entry> {
        if let Field::Job(j) = pick.field {
            return self.job_entries(pick, j);
        }
        match pick.field {
            Field::Skills => return self.skill_entries(pick),
            Field::Template => return self.template_entries(pick),
            Field::ExtraSkill => return self.extra_skill_entries(pick),
            _ => {}
        }
        let current = match pick.app {
            Some(_) => String::new(),
            None => self.own(pick.scope, pick.row, pick.field),
        };
        let filter = pick.filter.to_lowercase();
        let mut out = Vec::new();
        let mut entry = |name: &str, detail: String, picks: Option<Picked>| {
            let shown = match &picks {
                None => filter.is_empty(),
                Some(Picked::Typed) => true,
                Some(_) => name.to_lowercase().contains(&filter),
            };
            let model = matches!(pick.field, Field::Model | Field::Plan);
            let current = match &picks {
                None => false,
                Some(Picked::Value(v)) if v.is_empty() => current.is_empty(),
                Some(_) if model => app::canonical(name) == app::canonical(&current),
                Some(_) => name == current,
            };
            let mark = match &picks {
                Some(Picked::Value(m)) if model => self.mark_of(pick, m),
                _ => None,
            };
            let dim = matches!(&picks, Some(Picked::App(a)) if !self.is_installed(a));
            if shown {
                out.push(Entry {
                    name: name.to_string(),
                    detail,
                    current,
                    mark,
                    dim,
                    picks,
                });
            }
        };
        // A label's own field may fall through; a model after a new App may not.
        if pick.scope != Scope::Repo && pick.app.is_none() {
            let (name, whose) = match pick.scope {
                Scope::Extra => ("the Review's row", "the Review's"),
                _ => ("repo's row", "the repo's"),
            };
            let fell = self.fallback(pick.scope, pick.row, pick.field);
            let detail = format!("empty: {whose} {fell}");
            entry(name, detail, Some(Picked::Value(String::new())));
        }
        // A row on an App no longer in the table still lists the Apps.
        match (pick.field, self.pick_app(pick)) {
            (Field::App, _) => {
                for a in &APPS {
                    // Not installed, an App is greyed too.
                    let detail = match (self.is_installed(a), a.experimental) {
                        (_, true) => "experimental, unverified",
                        (false, false) => "not installed",
                        (true, false) => a.family,
                    };
                    entry(a.name, detail.into(), Some(Picked::App(a)));
                }
            }
            (_, None)
            | (
                Field::Same
                | Field::Job(_)
                | Field::Skills
                | Field::Template
                | Field::ExtraSkill
                | Field::Switch(_)
                | Field::Number(_)
                | Field::Bot(_),
                _,
            ) => {}
            (Field::Plan, Some(app)) => {
                for (id, _) in self.listed(app) {
                    entry(
                        id,
                        family_label(app, id).into(),
                        Some(Picked::Value(id.clone())),
                    );
                }
                let detail = "probed before it saves".to_string();
                entry("type an id…", detail, Some(Picked::Typed));
            }
            (Field::Model, Some(app)) => {
                let value = |m: &str| Some(Picked::Value(m.to_string()));
                if ROWS[pick.row].key == IF_LIMITED {
                    let detail = "no fallback".to_string();
                    entry("none", detail, value("none"));
                }
                // A split needs a named model for each half.
                if pick.scope != Scope::Repo
                    || pick.row != 0
                    || pick.app.is_some()
                    || self.split().is_none()
                {
                    let detail = format!("{}'s own · {}", app.name, family_label(app, "default"));
                    entry("default", detail, value("default"));
                }
                for (id, _) in self.listed(app) {
                    entry(id, family_label(app, id).into(), value(id));
                }
                let detail = "probed before it saves".to_string();
                entry("type an id…", detail, Some(Picked::Typed));
            }
            (Field::Effort, Some(app)) => {
                let model = self.row_value(pick, Field::Model);
                let levels =
                    std::iter::once("default".to_string()).chain(self.efforts(app, &model));
                for level in levels {
                    let detail = if level == "default" { "no flag" } else { "" };
                    entry(
                        &level,
                        detail.to_string(),
                        Some(Picked::Value(level.clone())),
                    );
                }
            }
        }
        out
    }

    /// A job's pick list: its suggestions as the job's App has them, one it
    /// cannot have left out; none; then every other skill that App loads or
    /// has built in. Filtered as entries are.
    fn job_entries(&self, pick: &Pick, j: usize) -> Vec<Entry> {
        let (job, suggestions) = JOBS[j];
        let current = self.manifest.pick(job);
        let filter = pick.filter.to_lowercase();
        let mut out = Vec::new();
        let mut entry = |name: &str, detail: String, mark, picks: Option<Picked>| {
            let shown = match &picks {
                None => filter.is_empty(),
                Some(_) => name.to_lowercase().contains(&filter),
            };
            if shown {
                out.push(Entry {
                    name: name.to_string(),
                    detail,
                    current: matches!(&picks, Some(Picked::Value(v)) if v == current),
                    mark,
                    dim: false,
                    picks,
                });
            }
        };
        let value = |name: &str| Some(Picked::Value(name.to_string()));
        let app = self.app(pick.row);
        let mut listed = vec![NONE.to_string()];
        if let Some(app) = app {
            entry("SUGGESTED", String::new(), None, None);
            for &(name, source) in suggestions.iter() {
                // an empty source is built in or Shipped: listed only when had
                if name == NONE || (source.is_empty() && self.have(app, name).is_none()) {
                    continue;
                }
                let named = name.to_string();
                match self.have(app, &named) {
                    Some((mark, color, detail)) => {
                        entry(&named, detail, Some((mark, color)), value(&named))
                    }
                    None => {
                        let detail = parse_source(source)
                            .map_or(String::new(), |s| short(&s.repo).to_string());
                        let mark = Some(("not installed", ORANGE));
                        entry(name, detail, mark, Some(Picked::Install(name, source)));
                    }
                }
                listed.push(named);
            }
        }
        let own = "the Stage skill's own instructions".to_string();
        entry(NONE, own, None, value(NONE));
        let Some(app) = app else {
            return out;
        };
        let names = app
            .built_in
            .iter()
            .copied()
            .chain(self.seen(app).into_iter().map(|(n, _)| n));
        let others: Vec<String> = distinct(names.map(String::from))
            .into_iter()
            .filter(|name| !listed.contains(name))
            .collect();
        if !others.is_empty() {
            let heading = format!("YOUR OTHER SKILLS {} CAN SEE", app.name.to_uppercase());
            entry(&heading, String::new(), None, None);
        }
        for name in others {
            if let Some((mark, color, detail)) = self.have(app, &name) {
                entry(&name, detail, Some((mark, color)), value(&name));
            }
        }
        out
    }

    /// The rule model, picked from pick's list, would break.
    fn mark_of(&self, pick: &Pick, model: &str) -> Option<(&'static str, Color)> {
        let key = ROWS[pick.row].key;
        let mut fields = vec![(pick.field, model.to_string())];
        if let Some(app) = pick.app {
            fields.insert(0, (Field::App, app.name.to_string()));
        }
        let mut doc = self.doc.clone();
        match pick.scope {
            Scope::Repo => put(&mut doc, key, &fields),
            // no rule reads an Extra review's row
            Scope::Extra => return None,
            Scope::Label => {
                let name = self.label.as_deref().unwrap_or_default();
                let labels = doc["labels"].as_object_mut()?;
                put_override(labels, name, Scope::Label, key, &fields).ok()?;
            }
        }
        let broken = broken_by(&self.doc, &doc)?;
        let mark = match (broken.rows, key) {
            // Only the rules' unknown-family texts say "cannot".
            _ if broken.text.contains("cannot") => "? family unknown",
            ([_, "review"], "implement") => "✗ the Review's model",
            ([_, _], "implement") => "✗ the fallback's model",
            (["implement", _], _) => "✗ Implement's model",
            (_, "side_a") => "✗ side B's family",
            _ => "✗ side A's family",
        };
        Some((mark, RED))
    }

    pub(crate) fn choices(&self, pick: &Pick) -> Vec<Picked> {
        self.entries(pick)
            .into_iter()
            .filter_map(|e| e.picks)
            .collect()
    }

    /// The foot's note on a setting, or on its open pick list.
    pub(crate) fn note_of(&self, row: usize, field: Field) -> String {
        if self.label.is_some() {
            return self.labels_note();
        }
        let tail = match field {
            Field::Job(j) => return format!(
                "The skill the Stage uses for {}: one not installed is installed first, as orqa-<name>; none leaves the Stage skill's own instructions. Your personal skills are listed once turned on, on the Skills page.",
                job_name(j)
            ),
            Field::Skills | Field::Template | Field::ExtraSkill => return self.labels_note(),
            Field::Switch(switch) if *switch == RELEASE_ON => "Enter or Space turns it on or off, saved at once, uncommitted.",
            Field::Switch(switch) if *switch == AGENT_MERGE => "Enter or Space turns it on or off, saved at once, uncommitted; only with PR comments and failing checks opened by themselves. On, the Orchestrator merges a Ticket's PR once its checks are green, the ticked review bots are done and every PR comment is fixed or answered; security, db and infra Tickets still wait for you.",
            Field::Bot(_) => "A review bot this repo has: Agent merge waits for its review. Enter or Space ticks it, saved at once, uncommitted.",
            Field::Switch(_) => "Enter or Space turns it on or off, saved at once, uncommitted; off, the poll only says it in a line and the command still works by hand.",
            Field::Number(n) => return number_note(n),
            Field::App => "Changing the App leads into its model list; the pair saves together.",
            Field::Same => match self.split() {
                Some(plan) => {
                    let model = self.value(row, Field::Model);
                    return format!("Off: plans on {plan}, implements on {model}. Enter or Space turns it on: one model plans and implements.");
                }
                None => return "On: one model plans and implements. Enter or Space turns it off to plan on another model of Implement's family.".to_string(),
            },
            Field::Plan if self.pick.is_some() && self.split().is_none() => {
                return "Pick the plan's model to split planning from implementing; Esc keeps one model for both.".to_string();
            }
            Field::Plan => {
                return "Plans in plan mode, on a model of Implement's family; Implement's own model plans on one model again.".to_string();
            }
            _ => "Default passes no flag.",
        };
        format!("{} {tail}", ROWS[row].note)
    }

    /// The Skills page's rows past the location: the installed skills.
    pub(crate) fn skill_names(&self) -> Vec<String> {
        self.manifest.skills.keys().cloned().collect()
    }

    /// The foot's note on the Skills page's row i: where the skills live,
    /// your personal skills' switch, or a skill.
    pub(crate) fn skills_note(&self, i: usize) -> String {
        let names = self.skill_names();
        let Some(name) = i.checked_sub(SKILL_ROWS).map(|i| &names[i]) else {
            return match i {
                0 => "Committed: a skill change here takes effect for Tickets once it is merged.",
                _ => "Enter or Space turns on or off, for you alone, the jobs picking and the Stages loading your own skills: ~/.claude/skills, ~/.agents/skills and Claude Code plugins'. Off, only Orqadence's, the repo's own and the Apps' built-in ones.",
            }
            .to_string();
        };
        match self.manifest.skills[name].shipped {
            true => format!("{name} is a Shipped skill: orqa init installs and updates it, and it cannot be removed."),
            false => format!("u updates {name} from its source, d removes it; a adds a skill from a source, U updates every one."),
        }
    }

    /// The foot's note on the TypeSafe page's row i.
    pub(crate) fn typesafe_note(&self, i: usize) -> String {
        let floor = "Enter types a number from 0 to 1, or nothing for the default, saved at once, uncommitted: the next Judgment reads it.";
        match i {
            0 => "Judges a Wake's next step, a Plan's approval and a Finding the Debate still disputes; Enter turns it on or off.".to_string(),
            1 => format!("Kept in {}, readable only by you; TYPESAFE_API_KEY in the environment wins over it.", setup::KEY_FILE),
            2 => format!("At or above it a Wake's Judgment acts; below it the Wake is a Question. {floor}"),
            _ => format!("A Plan whose covers and in scope reach it, and that asks nothing, is approved; otherwise it is a Question. {floor}"),
        }
    }

    /// The foot's note on the On call page's row i.
    pub(crate) fn on_call_note(&self, i: usize) -> String {
        match i {
            0 => format!("Kept in {}, readable only by you; {} in the environment wins over it. Enter types a new one; nothing clears it: On call off.", on_call::CONFIG, on_call::TOKEN_VAR),
            1 => format!("How long a Question waits before On call starts. Enter types a whole number of at least 1, or nothing for the default {DEFAULT_MINUTES}, saved at once."),
            _ => "Enter rings your phone through Moshi with a test push.".to_string(),
        }
    }

    /// A whole number as a page shows it: its number, and whether that is
    /// the default; or config.json's value as written, when it breaks its
    /// rule.
    pub(crate) fn count(&self, count: &Count) -> Result<(usize, bool), String> {
        let set = &self.doc[count.key];
        app::count_in(&self.doc, count)
            .map(|n| (n, set.is_null() || set == ""))
            .map_err(|_| set.to_string())
    }

    /// A floor as the TypeSafe page shows it: its number, and whether that
    /// is the default; or config.json's value as written, when it is not a
    /// number from 0 to 1.
    pub(crate) fn floor(&self, floor: &Floor) -> Result<(f64, bool), String> {
        let set = &self.doc[floor.key];
        app::floor_in(&self.doc, floor)
            .map(|value| (value, set.is_null() || set == ""))
            .map_err(|_| set.to_string())
    }

    /// config.json's labels, each entry by its name as read, or why it
    /// cannot be.
    pub(crate) fn labels(&self) -> BTreeMap<String, Result<Label, String>> {
        app::labels(&self.doc)
    }

    /// The names of config.json's labels, sorted.
    pub(crate) fn label_names(&self) -> Vec<String> {
        self.labels().into_keys().collect()
    }

    /// A label's entry as read, or why it cannot be.
    pub(crate) fn label_of(&self, name: &str) -> Result<Label, String> {
        self.labels()
            .remove(name)
            .unwrap_or_else(|| Err(format!("no label orqa:{name}")))
    }

    /// The Labels line on the left: "2 labels".
    pub(crate) fn labels_summary(&self) -> String {
        plural(self.labels().len(), "label")
    }

    /// The foot's note on the Labels page: the list's keys, or the open
    /// label's row under the cursor.
    pub(crate) fn labels_note(&self) -> String {
        if self.label.is_none() {
            return "a adds a label, e or Enter opens it, r renames it (its PR template mapping follows), d deletes it (its PR template file stays). Every change saves at once, uncommitted, to .orqadence/config.json.".to_string();
        }
        match self.label_items().get(self.setting) {
            Some(LabelItem::Kind) => "area: the one type of work a Ticket does, one per Ticket; modifier: only changes rows, and combines with an area. Enter or Space toggles.".to_string(),
            Some(LabelItem::Skills) => "The skills its code-editing Stages load, from the ones installed: Enter picks them, or types a source to install one first.".to_string(),
            Some(LabelItem::Template) => "The PR template Fix writes from: default, a file of .github/PULL_REQUEST_TEMPLATE, or new, a copy of the default with a section for this label.".to_string(),
            Some(LabelItem::ExtraSkill) => "The review skill of this label's Extra review, from the ones installed; none: no Extra review. It runs the Review's Stage skill with it.".to_string(),
            Some(LabelItem::Position) => "When the Extra review runs: every Round, after the Review; the first Round only; or before the PR, after the last Round. Enter or Space cycles.".to_string(),
            Some(LabelItem::Debate) => "on: its Findings join the Debate; off: they go straight to the Fix as fix items, not debated. Enter or Space toggles.".to_string(),
            Some(LabelItem::Extra(_)) => "The Extra review's App, model and effort: an empty field falls back to the Review's row. Enter picks.".to_string(),
            Some(LabelItem::Row(r, _)) => format!("Overrides the repo's {} row for a Ticket with this label: an empty field falls through to the repo's. Enter picks; 'repo's row' empties it.", ROWS[*r].name),
            _ => "One line given to its code-editing Stages as an Input. Enter types it; nothing clears it.".to_string(),
        }
    }

    /// The open label's page, row by row: kind, skills, guidance and PR
    /// template; an Area label's Extra review; then the Stage rows it
    /// overrides, all but the Release's. A label that cannot be read has the
    /// first three only.
    pub(crate) fn label_items(&self) -> Vec<LabelItem> {
        use LabelItem::{
            Debate, Extra, ExtraSkill, Guidance, Kind, Position, Row, Skills, Template,
        };
        let mut items = vec![Kind, Skills, Guidance];
        let Some(Ok(label)) = self.label.as_deref().map(|name| self.label_of(name)) else {
            return items;
        };
        items.push(Template);
        if label.kind == "area" {
            items.extend([ExtraSkill, Position, Debate]);
            items.extend(FIELDS.map(Extra));
        }
        let rows = (0..ROWS.len()).filter(|&r| ROWS[r].key != RELEASE);
        items.extend(rows.flat_map(|r| FIELDS.map(|f| Row(r, f))));
        items
    }

    /// A label name as typed, as bd takes a label: not empty, no whitespace,
    /// no comma (bd's list separator), and not one config.json has.
    fn valid_label(&self, typed: &str) -> Result<String, String> {
        let name = label_name(typed);
        if name.is_empty() {
            return Err("Refused: no name after orqa:. Nothing changed.".to_string());
        }
        if name.chars().any(|c| c.is_whitespace() || c == ',') {
            return Err(format!(
                "Refused: '{name}' is not a bd label: no spaces or commas. Nothing changed."
            ));
        }
        if self.doc["labels"].get(name).is_some() {
            return Err(format!(
                "Refused: orqa:{name} is there already. Nothing changed."
            ));
        }
        Ok(name.to_string())
    }

    /// The open label's Extra review skill list: none, then each skill
    /// installed but the Stage skills. Filtered as entries are.
    fn extra_skill_entries(&self, pick: &Pick) -> Vec<Entry> {
        let name = self.label.as_deref().unwrap_or_default();
        let on = self
            .label_of(name)
            .map(|l| l.extra_review.skill)
            .unwrap_or_default();
        let filter = pick.filter.to_lowercase();
        let skills = self.manifest.skills.iter();
        // The Stage skills and create-pr are not reviews.
        let listed = skills.filter(|(name, _)| {
            !name.starts_with("orqa-stage-") && *name != crate::skills::CREATE_PR
        });
        let mut out = Vec::new();
        if NONE.contains(&filter) {
            out.push(Entry {
                name: NONE.to_string(),
                detail: "no Extra review".to_string(),
                current: on.is_empty(),
                mark: None,
                dim: false,
                picks: Some(Picked::Value(String::new())),
            });
        }
        out.extend(
            listed
                .filter(|(name, _)| name.to_lowercase().contains(&filter))
                .map(|(name, skill)| Entry {
                    name: name.clone(),
                    detail: match skill.shipped {
                        true => "shipped with Orqadence".to_string(),
                        false => short(&skill.repo).to_string(),
                    },
                    current: *name == on,
                    mark: None,
                    dim: false,
                    picks: Some(Picked::Value(name.clone())),
                }),
        );
        out
    }

    /// The open label's PR template list: default, each file of the
    /// template directory, then new. Filtered as entries are.
    fn template_entries(&self, pick: &Pick) -> Vec<Entry> {
        let name = self.label.as_deref().unwrap_or_default();
        let on = self
            .label_of(name)
            .map(|l| l.pr_template)
            .unwrap_or_default();
        let filter = pick.filter.to_lowercase();
        let entry = |name: &str, detail: &str, mark, current, picks| Entry {
            name: name.to_string(),
            detail: detail.to_string(),
            current,
            mark,
            dim: false,
            picks: Some(picks),
        };
        let mut out = Vec::new();
        if "default".contains(&filter) {
            let default = Picked::Value(String::new());
            out.push(entry(
                "default",
                "the repo's default template",
                None,
                on.is_empty(),
                default,
            ));
        }
        let mut files = self.templates.clone();
        // a mapped file that has gone falls back to the default at PR time
        if !on.is_empty() && !files.contains(&on) {
            files.push(on.clone());
        }
        for file in files.iter().filter(|f| f.to_lowercase().contains(&filter)) {
            let gone = !self.templates.contains(file);
            let mark = gone.then_some(("not found", ORANGE));
            let picked = Picked::Value(file.clone());
            out.push(entry(file, setup::TEMPLATE_DIR, mark, *file == on, picked));
        }
        let shipped = setup::LABELS
            .iter()
            .any(|l| l.name == name && !l.pr_section.is_empty());
        let detail = match shipped {
            true => "the default plus its shipped section",
            false => "the default plus a section, its heading typed",
        };
        out.push(entry("new…", detail, None, false, Picked::Typed));
        out
    }

    /// The open label's skills pick list: each skill installed but the
    /// Shipped ones, those on the label marked, then 'from a source…'.
    /// Filtered as entries are.
    fn skill_entries(&self, pick: &Pick) -> Vec<Entry> {
        let on = self
            .label
            .as_deref()
            .and_then(|name| self.label_of(name).ok())
            .map(|label| label.skills)
            .unwrap_or_default();
        let filter = pick.filter.to_lowercase();
        let mut out: Vec<Entry> = self
            .manifest
            .skills
            .iter()
            .filter(|(name, skill)| !skill.shipped && name.to_lowercase().contains(&filter))
            .map(|(name, skill)| Entry {
                name: name.clone(),
                detail: short(&skill.repo).to_string(),
                current: on.contains(name),
                mark: on.contains(name).then_some(("on", GREEN)),
                dim: false,
                picks: Some(Picked::Value(name.clone())),
            })
            .collect();
        out.push(Entry {
            name: "from a source…".to_string(),
            detail: "cloned and installed, then on the label".to_string(),
            current: false,
            mark: None,
            dim: false,
            picks: Some(Picked::Typed),
        });
        out
    }
}

/// A label name as typed: trimmed, a typed orqa: dropped.
fn label_name(typed: &str) -> &str {
    let name = typed.trim();
    name.strip_prefix("orqa:").unwrap_or(name)
}

/// A label's summary on the page: its skills, whether it has guidance and
/// an Extra review.
pub(crate) fn label_line(label: &Label) -> String {
    let mut parts = Vec::new();
    if !label.skills.is_empty() {
        parts.push(format!("skills: {}", label.skills.join(", ")));
    }
    if !label.guidance.is_empty() {
        parts.push("guidance".to_string());
    }
    if !label.extra_review.skill.is_empty() {
        parts.push("extra review".to_string());
    }
    parts.join(" · ")
}

/// The entry under name in labels, to change; a missing one, one that is
/// not an object, or one a Stage could not read, refuses.
fn label_entry<'a>(
    labels: &'a mut serde_json::Map<String, Value>,
    name: &str,
) -> Result<&'a mut Value, String> {
    match labels.get_mut(name) {
        Some(entry) if entry.is_object() => {
            app::entry(name, entry)?;
            Ok(entry)
        }
        Some(_) => Err(format!("labels {name} in config.json is not an object")),
        None => Err(format!("config.json has no label orqa:{name}")),
    }
}

/// A label's skills changed in place; a missing or null skills starts
/// empty (label_entry refused one that is not a list of strings).
fn put_skills(
    labels: &mut serde_json::Map<String, Value>,
    name: &str,
    change: impl FnOnce(&mut Vec<String>),
) -> Result<(), String> {
    let entry = label_entry(labels, name)?;
    let mut skills: Vec<String> =
        serde_json::from_value(entry["skills"].take()).unwrap_or_default();
    change(&mut skills);
    entry["skills"] = json!(skills);
    Ok(())
}

/// A label's Extra review entry, made with the new-label defaults when it
/// has none.
fn extra_mut(entry: &mut Value) -> &mut Value {
    if !entry["extra_review"].is_object() {
        entry["extra_review"] = json!({"position": "every", "debate": true});
    }
    &mut entry["extra_review"]
}

/// A label's override of a row's fields, or its Extra review's: each
/// non-empty value put in, an empty one removed, and a row or rows left
/// empty with them.
fn put_override(
    labels: &mut serde_json::Map<String, Value>,
    name: &str,
    scope: Scope,
    key: &str,
    fields: &[(Field, String)],
) -> Result<(), String> {
    let entry = label_entry(labels, name)?;
    let holder = match scope {
        Scope::Extra => extra_mut(entry),
        _ => {
            if !entry["rows"].is_object() {
                entry["rows"] = json!({});
            }
            &mut entry["rows"][key]
        }
    };
    if !holder.is_object() {
        *holder = json!({});
    }
    for (field, value) in fields {
        match value.is_empty() {
            true => _ = holder.as_object_mut().unwrap().remove(field.key()),
            false => holder[field.key()] = json!(value),
        }
    }
    // An App or model of its own plans on one model, as put's new App
    // does: the repo's split may not fit it.
    let moved = fields
        .iter()
        .any(|(field, _)| matches!(field, Field::App | Field::Model));
    if scope == Scope::Label && key == "implement" && moved {
        let row = holder.as_object_mut().unwrap();
        match row.contains_key("app") || row.contains_key("model") {
            true => _ = row.insert(Field::Plan.key().into(), json!("default")),
            false => _ = row.remove(Field::Plan.key()),
        }
    }
    if scope != Scope::Extra {
        let rows = entry["rows"].as_object_mut().unwrap();
        rows.retain(|_, row| row.as_object().is_none_or(|fields| !fields.is_empty()));
        if rows.is_empty() {
            entry.as_object_mut().unwrap().remove("rows");
        }
    }
    Ok(())
}

/// What an App said as it refused a probe: the last line it printed,
/// stdout first (claude -p prints its error there, under a warning on
/// stderr), a JSON error line's message (codex exec's) pulled out.
fn refusal(err: &RunError) -> String {
    let last = |text: &str| {
        text.lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .map(String::from)
    };
    let line = last(&err.stdout)
        .or_else(|| last(&err.stderr))
        .unwrap_or_else(|| err.status.clone());
    let json = line
        .find('{')
        .and_then(|i| serde_json::from_str::<Value>(&line[i..]).ok());
    let message = json.and_then(|v| {
        v["error"]["message"]
            .as_str()
            .or(v["message"].as_str())
            .map(String::from)
    });
    message.unwrap_or(line).trim_end_matches('.').to_string()
}

/// config.json read afresh: as read, with a row's fields put in, and the
/// row as the Stage that starts on it will read it.
fn staged(
    repo: &Path,
    key: &str,
    fields: &[(Field, String)],
) -> Result<(PathBuf, Value, Value, Row), String> {
    let (path, read) = app::read_object(repo)?;
    let mut doc = read.clone();
    put(&mut doc, key, fields);
    let row = app::row_in(&doc, key, &path)?;
    if let Some(broken) = broken_by(&read, &doc) {
        return Err(broken.text);
    }
    Ok((path, read, doc, row))
}

/// config.json read afresh with a label's override put in, as
/// put_override, and the row as the Stage that starts on it will read it;
/// a change that breaks a rule refuses.
fn staged_override(
    repo: &Path,
    name: &str,
    scope: Scope,
    key: &str,
    fields: &[(Field, String)],
) -> Result<(PathBuf, Value, Row), String> {
    let (path, read) = app::read_object(repo)?;
    let mut doc = read.clone();
    let labels = doc["labels"]
        .as_object_mut()
        .ok_or_else(|| "labels in config.json is not an object".to_string())?;
    put_override(labels, name, scope, key, fields)?;
    if let Some(broken) = broken_by(&read, &doc).filter(|_| scope == Scope::Label) {
        return Err(broken.text);
    }
    let view = match scope {
        Scope::Extra => app::with_extra(&doc, name),
        _ => app::with_label(&doc, name),
    };
    let row = app::row_in(&view, key, &path)?;
    Ok((path, doc, row))
}

/// The first rule the change from before to after breaks: one broken after
/// that held before, or was not checked, so a config.json broken by hand
/// mends one rule at a time.
fn broken_by(before: &Value, after: &Value) -> Option<Check> {
    let was = app::checks(before);
    let was_broken = |c: &Check| {
        was.iter()
            .any(|w| w.rows == c.rows && w.label == c.label && !w.holds)
    };
    app::checks(after)
        .into_iter()
        .find(|c| !c.holds && !was_broken(c))
}

/// doc with a row's fields put in. On Implement a new App, or a plan model
/// that is default or Implement's own, plans on one model again, so an App
/// change never strands the plan; a split names both halves by full id, as
/// opusplan's remap needs. A field not a string is left for row_in to refuse.
pub(crate) fn put(doc: &mut Value, key: &str, fields: &[(Field, String)]) {
    if !doc[key].is_object() {
        doc[key] = json!({});
    }
    for (field, value) in fields {
        doc[key][field.key()] = json!(value);
    }
    if key != "implement" {
        return;
    }
    let (Ok(model), Ok(plan)) = (
        app::field(doc, key, Field::Model.key()),
        app::field(doc, key, Field::Plan.key()),
    ) else {
        return;
    };
    let row = doc[key].as_object_mut().unwrap();
    let new_app = fields.iter().any(|(field, _)| *field == Field::App);
    if new_app || plan == "default" || app::canonical(&plan) == app::canonical(&model) {
        row.remove(Field::Plan.key());
    } else {
        row.insert(Field::Model.key().into(), json!(app::full_id(&model)));
        row.insert(Field::Plan.key().into(), json!(app::full_id(&plan)));
    }
}

impl Screen {
    /// /config: reads config.json, each App's models, the Skill manifest
    /// and the skills you have. An unreadable config.json, or one not an
    /// object, is a notice: nothing may save over it. A garbled manifest
    /// shows no skills and says why; each change to it reads it afresh and
    /// refuses.
    // ponytail: the lists, `which` and claude plugin list run on the screen
    // thread (codex's bundled catalog takes ~10 ms); a thread when one is slow.
    pub(super) fn open_config(&mut self) {
        let repo = &self.cfg.repo;
        let doc = match app::read_object(repo) {
            Ok((_, doc)) => doc,
            Err(err) => return self.notice(&err, NOTICE_WINDOW),
        };
        let (manifest, note) = match Manifest::load(repo) {
            Ok(manifest) => (manifest, None),
            Err(err) => (Manifest::default(), Some((err, RED))),
        };
        let tools = &*self.cfg.tools;
        let installed = APPS
            .iter()
            .map(|a| {
                tools.run(repo, &["which", a.bin]).ok()?;
                let version = tools.run(repo, &[a.bin, "--version"]).unwrap_or_default();
                Some(
                    version
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                )
            })
            .collect();
        self.settings = Some(Settings {
            doc,
            models: APPS.iter().map(|a| (a.models)(tools, repo)).collect(),
            installed,
            manifest,
            found: found(repo, &self.cfg.home, tools),
            personal: manifest::personal(repo),
            section: 0,
            open: false,
            setting: 0,
            pick: None,
            typing: None,
            probe: None,
            probed: Vec::new(),
            busy: None,
            listing: None,
            confirm: None,
            label: None,
            templates: setup::template_files(&repo.join(setup::TEMPLATE_DIR)),
            note,
            saved: None,
        });
    }

    /// A key while /config is open. A probe or a clone takes none but Esc,
    /// which stops waiting on it; a question takes y or n; typing takes the
    /// line; a checklist or a pick list moves and picks; the left list and a
    /// page move and open, Esc going back and then closing.
    pub(super) fn config_key(&mut self, code: KeyCode, held: bool) {
        let st = self.settings.as_mut().unwrap();
        st.note = None;
        if st.probe.is_some() {
            if code == KeyCode::Esc {
                st.probe = None;
                st.note = Some(("Probe dropped: nothing changed.".to_string(), MUTED));
            }
            return;
        }
        // ponytail: no key stops waiting on a clone, which saves the manifest
        // when done and would overwrite a change made meanwhile; a clone that
        // hangs holds /config until git gives up (Ctrl-C still leaves).
        if st.busy.is_some() {
            return;
        }
        if st.confirm.is_some() {
            match code {
                KeyCode::Char('y') => {
                    let (_, confirm) = st.confirm.take().unwrap();
                    match confirm {
                        Confirm::Remove(name) => self.remove_skill(&name),
                        Confirm::TypeSafeOff => self.typesafe_to(false),
                        Confirm::DeleteLabel(name) => self.delete_label(&name),
                    }
                }
                KeyCode::Char('n') | KeyCode::Esc => st.confirm = None,
                _ => {}
            }
            return;
        }
        if let Some((_, text)) = &mut st.typing {
            match code {
                KeyCode::Esc => st.typing = None,
                KeyCode::Backspace => _ = text.pop(),
                KeyCode::Char(c) if !held => text.push(c),
                KeyCode::Enter => {
                    let (typing, text) = st.typing.take().unwrap();
                    match (typing, text.trim()) {
                        (Typing::Floor(floor), text) => self.keep_floor(floor, text.to_string()),
                        (Typing::Number(n), text) => self.keep_number(n, text.to_string()),
                        (Typing::Token, token) => self.keep_token(token.to_string()),
                        (Typing::Minutes, text) => self.keep_minutes(text.to_string()),
                        (Typing::Guidance, text) => self.keep_guidance(text.to_string()),
                        (_, "") => {
                            st.note = Some(("Nothing typed: nothing changed.".to_string(), MUTED))
                        }
                        (Typing::Model(pick), id) => self.pick_model(&pick, id),
                        (Typing::Source, source) => self.add_source(source.to_string()),
                        (Typing::Key, key) => self.keep_key(key.to_string()),
                        (Typing::LabelName, name) => self.add_label(name.to_string()),
                        (Typing::Heading, heading) => self.keep_heading(heading),
                        (Typing::Rename(old), name) => self.rename_label(old, name.to_string()),
                    }
                }
                _ => {}
            }
            return;
        }
        if let Some(listing) = &mut st.listing {
            let n = listing.names.len();
            match code {
                KeyCode::Up => listing.cursor = listing.cursor.saturating_sub(1),
                KeyCode::Down => listing.cursor = (listing.cursor + 1).min(n - 1),
                KeyCode::Char(' ') => {
                    let (_, installed, ticked) = &mut listing.names[listing.cursor];
                    *ticked = *installed || !*ticked;
                }
                KeyCode::Enter => self.install_ticked(),
                KeyCode::Esc => st.listing = None,
                _ => {}
            }
            return;
        }
        if let Some(pick) = &st.pick {
            let n = st.choices(pick).len();
            let pick = st.pick.as_mut().unwrap();
            match code {
                KeyCode::Up => pick.cursor = pick.cursor.saturating_sub(1),
                KeyCode::Down => pick.cursor = (pick.cursor + 1).min(n.saturating_sub(1)),
                KeyCode::Esc => st.pick = None,
                KeyCode::Backspace => {
                    pick.filter.pop();
                    pick.cursor = 0;
                }
                KeyCode::Char(c) if !held => {
                    pick.filter.push(c);
                    pick.cursor = 0;
                }
                KeyCode::Enter => {
                    let pick = st.pick.take().unwrap();
                    match st.choices(&pick).into_iter().nth(pick.cursor) {
                        Some(picked) => self.choose(pick, picked),
                        None => st.pick = Some(pick),
                    }
                }
                _ => {}
            }
            return;
        }
        if !st.open {
            match code {
                KeyCode::Up => st.section = st.section.saturating_sub(1),
                KeyCode::Down => st.section = (st.section + 1).min(ON_CALL_PAGE),
                KeyCode::Right | KeyCode::Enter => {
                    st.open = true;
                    st.setting = 0;
                }
                KeyCode::Esc => self.settings = None,
                _ => {}
            }
            return;
        }
        if st.section == APPS_PAGE {
            match code {
                KeyCode::Up => st.setting = st.setting.saturating_sub(1),
                KeyCode::Down => st.setting = (st.setting + 1).min(APPS.len() - 1),
                KeyCode::Left | KeyCode::Esc => st.open = false,
                _ => {}
            }
            return;
        }
        if st.section == SKILLS_PAGE {
            let names = st.skill_names();
            let skill = st.setting.checked_sub(SKILL_ROWS).map(|i| names[i].clone());
            match code {
                KeyCode::Up => st.setting = st.setting.saturating_sub(1),
                KeyCode::Down => st.setting = (st.setting + 1).min(names.len() + SKILL_ROWS - 1),
                KeyCode::Left | KeyCode::Esc => st.open = false,
                KeyCode::Enter | KeyCode::Char(' ') if st.setting == 1 => {
                    let on = !st.personal;
                    self.personal_to(on)
                }
                KeyCode::Char('a') => st.typing = Some((Typing::Source, String::new())),
                KeyCode::Char('U') => self.update_skills(None),
                KeyCode::Char('u') if skill.is_some() => self.update_skills(skill),
                KeyCode::Char('d') | KeyCode::Delete if skill.is_some() => {
                    self.ask_remove(skill.unwrap())
                }
                _ => {}
            }
            return;
        }
        if st.section == LABELS_PAGE {
            let names = st.label_names();
            let label = st.label.clone();
            let under = names.get(st.setting).cloned();
            match (label, under, code) {
                (_, _, KeyCode::Up) => st.setting = st.setting.saturating_sub(1),
                (None, _, KeyCode::Down) => {
                    st.setting = (st.setting + 1).min(names.len().saturating_sub(1))
                }
                (Some(_), _, KeyCode::Down) => {
                    st.setting = (st.setting + 1).min(st.label_items().len() - 1)
                }
                (None, _, KeyCode::Left | KeyCode::Esc) => st.open = false,
                (Some(label), _, KeyCode::Left | KeyCode::Esc) => {
                    st.label = None;
                    st.setting = names.iter().position(|n| *n == label).unwrap_or(0);
                }
                (None, _, KeyCode::Char('a')) => {
                    st.typing = Some((Typing::LabelName, String::new()))
                }
                (None, Some(name), KeyCode::Enter | KeyCode::Char('e')) => {
                    st.label = Some(name);
                    st.setting = 0;
                }
                (None, Some(name), KeyCode::Char('r')) => {
                    st.typing = Some((Typing::Rename(name.clone()), name));
                }
                (None, Some(name), KeyCode::Char('d') | KeyCode::Delete) => {
                    let text = format!("Delete orqa:{name}? A Ticket still carrying it wakes its next Stage; its PR template file stays.");
                    st.confirm = Some((text, Confirm::DeleteLabel(name)));
                }
                (Some(label), _, KeyCode::Enter | KeyCode::Char(' ')) => {
                    if let Some(&item) = st.label_items().get(st.setting) {
                        self.label_item(&label, item, code == KeyCode::Enter);
                    }
                }
                _ => {}
            }
            return;
        }
        if st.section == RUN_PAGE {
            let numbers: Vec<&'static Number> = run_numbers().collect();
            match code {
                KeyCode::Up => st.setting = st.setting.saturating_sub(1),
                KeyCode::Down => st.setting = (st.setting + 1).min(numbers.len() - 1),
                KeyCode::Left | KeyCode::Esc => st.open = false,
                KeyCode::Enter => {
                    st.typing = Some((Typing::Number(numbers[st.setting]), String::new()))
                }
                _ => {}
            }
            return;
        }
        if st.section == ON_CALL_PAGE {
            match code {
                KeyCode::Up => st.setting = st.setting.saturating_sub(1),
                KeyCode::Down => st.setting = (st.setting + 1).min(2),
                KeyCode::Left | KeyCode::Esc => st.open = false,
                KeyCode::Enter if st.setting == 0 => {
                    st.typing = Some((Typing::Token, String::new()))
                }
                KeyCode::Enter if st.setting == 1 => {
                    st.typing = Some((Typing::Minutes, String::new()))
                }
                KeyCode::Enter => self.test_push(),
                _ => {}
            }
            return;
        }
        if st.section == TYPESAFE_PAGE {
            let key = self.cfg.api_key.clone();
            match code {
                KeyCode::Up => st.setting = st.setting.saturating_sub(1),
                KeyCode::Down => st.setting = (st.setting + 1).min(1 + FLOORS.len()),
                KeyCode::Left | KeyCode::Esc => st.open = false,
                KeyCode::Enter if st.setting >= 2 => {
                    st.typing = Some((Typing::Floor(FLOORS[st.setting - 2]), String::new()))
                }
                KeyCode::Enter if key.is_empty() => st.typing = Some((Typing::Key, String::new())),
                KeyCode::Enter if st.setting == 1 => {}
                KeyCode::Enter if st.typesafe(&key) => {
                    st.confirm = Some((TYPESAFE_OFF.to_string(), Confirm::TypeSafeOff))
                }
                KeyCode::Enter => self.typesafe_to(true),
                _ => {}
            }
            return;
        }
        let items = st.items();
        match code {
            KeyCode::Up => st.setting = st.setting.saturating_sub(1),
            KeyCode::Down => st.setting = (st.setting + 1).min(items.len() - 1),
            KeyCode::Left | KeyCode::Esc => st.open = false,
            KeyCode::Enter | KeyCode::Char(' ') if items[st.setting].1 == Field::Same => {
                self.toggle()
            }
            KeyCode::Enter | KeyCode::Char(' ') => match items[st.setting] {
                (_, Field::Switch(switch)) => {
                    let on = !app::switch_in(&st.doc, switch);
                    self.switch_to(switch, on)
                }
                (_, Field::Bot(b)) => self.tick_bot(REVIEW_BOTS[b]),
                (_, Field::Number(n)) if code == KeyCode::Enter => {
                    st.typing = Some((Typing::Number(n), String::new()))
                }
                (row, field) if code == KeyCode::Enter => match st.app(row) {
                    Some(app) if field == Field::Effort && app.effort.is_empty() => {
                        let text = format!("{} has no effort flag.", app.name);
                        st.note = Some((text, MUTED));
                    }
                    _ => self.open_pick(row, field, None, Scope::Repo),
                },
                _ => {}
            },
            _ => {}
        }
    }

    /// 'Same model for plan and implementation': on, a split turns it off
    /// through the plan's model list, refused off claude (opusplan) or with
    /// Implement's model at default; off, turning it on saves at once.
    fn toggle(&mut self) {
        let st = self.settings.as_mut().unwrap();
        if st.split().is_some() {
            return self.change(0, vec![(Field::Plan, "default".to_string())]);
        }
        let refused = match st.app(0) {
            Some(app) if app.name != "claude" => format!(
                "Refused: {} cannot plan on one model and implement on another: only claude splits, through opusplan. Nothing changed.",
                app.name
            ),
            _ if st.value(0, Field::Model) == "default" => {
                "Pick Implement's model first: the split needs a named model for each half."
                    .to_string()
            }
            _ => return self.open_pick(0, Field::Plan, None, Scope::Repo),
        };
        st.note = Some((refused, RED));
    }

    /// Opens the pick list for a row's setting, the cursor on its value. A
    /// model list whose App could not list its models raises a red Notice
    /// modal with the App's error over it.
    fn open_pick(&mut self, row: usize, field: Field, app: Option<&'static App>, scope: Scope) {
        let st = self.settings.as_mut().unwrap();
        if field == Field::Template {
            st.templates = setup::template_files(&self.cfg.repo.join(setup::TEMPLATE_DIR));
        }
        let mut pick = Pick {
            scope,
            row,
            field,
            app,
            cursor: 0,
            filter: String::new(),
        };
        pick.cursor = st
            .entries(&pick)
            .iter()
            .filter(|e| e.picks.is_some())
            .position(|e| e.current)
            .unwrap_or(0);
        let error = match (field, st.pick_app(&pick)) {
            (Field::Model | Field::Plan, Some(app)) => st
                .catalog(app)
                .as_ref()
                .err()
                .map(|err| format!("{} could not list its models: {err}", app.name)),
            _ => None,
        };
        st.pick = Some(pick);
        if let Some(text) = error {
            self.notify(NoticeKind::Error, &text, None);
        }
    }

    /// A pick list's choice: a new App leads into its model list, one not
    /// on PATH says where to get it, one a row cannot run on is refused; a
    /// model or an effort changes the row.
    fn choose(&mut self, pick: Pick, picked: Picked) {
        let st = self.settings.as_mut().unwrap();
        match picked {
            Picked::App(a) if !st.is_installed(a) => st.note = Some((not_on_path(a), MUTED)),
            Picked::App(a) if st.own(pick.scope, pick.row, Field::App) == a.name => {}
            Picked::App(a) => match app::runs_on(ROWS[pick.row].key, a) {
                Ok(()) => self.open_pick(pick.row, Field::Model, Some(a), pick.scope),
                Err(err) => st.note = Some((format!("Refused: {err}. Nothing changed."), RED)),
            },
            Picked::Typed if pick.field == Field::Skills => {
                st.typing = Some((Typing::Source, String::new()))
            }
            Picked::Typed if pick.field == Field::Template => self.new_template(),
            Picked::Typed => st.typing = Some((Typing::Model(pick), String::new())),
            Picked::Install(name, source) => {
                if let Field::Job(j) = pick.field {
                    self.install_for(j, name, source)
                }
            }
            Picked::Value(value) => match pick.field {
                Field::Job(j) => {
                    self.set_pick(j, &value);
                }
                Field::Skills => self.toggle_skill(pick, &value),
                Field::Template => self.pick_template(&value),
                Field::ExtraSkill => self.pick_extra_skill(&value),
                Field::Model | Field::Plan => self.pick_model(&pick, &value),
                field => self.change_in(pick.scope, pick.row, vec![(field, value)]),
            },
        }
    }

    /// A model picked or typed: with the new App it came after, the pair;
    /// the effort back to default on a new App, or when the model does not
    /// list it.
    fn pick_model(&mut self, pick: &Pick, model: &str) {
        let st = self.settings.as_ref().unwrap();
        let mut fields = vec![(pick.field, model.to_string())];
        if let Some(app) = st.pick_app(pick).filter(|_| pick.field == Field::Model) {
            if pick.app.is_some() {
                fields.insert(0, (Field::App, app.name.to_string()));
            }
            let effort = st.row_value(pick, Field::Effort);
            if effort != "default"
                && (pick.app.is_some() || !st.efforts(app, model).contains(&effort))
            {
                fields.push((Field::Effort, "default".to_string()));
            }
        }
        self.change_in(pick.scope, pick.row, fields);
    }

    /// A change to a row: refused if the Stage could not start on it; one
    /// that changes nothing is dropped; a named model is probed first, once
    /// per App while /config is open, anything else saves at once.
    // ponytail: no timeout on the probe: Esc drops one that hangs, its
    // process running on to its end (it has no stdin to wait on).
    fn change(&mut self, row: usize, fields: Vec<(Field, String)>) {
        let repo = self.cfg.repo.clone();
        let st = self.settings.as_mut().unwrap();
        let key = ROWS[row].key;
        let (now, doc, staged) = match staged(&repo, key, &fields) {
            Ok((_, now, doc, staged)) => (now, doc, staged),
            Err(err) => {
                st.note = Some((format!("Refused: {err}. Nothing changed."), RED));
                return;
            }
        };
        let same =
            |(field, v): &(Field, String)| app::field(&now, key, field.key()).as_ref() == Ok(v);
        if doc == now || fields.iter().all(same) {
            return;
        }
        // The model picked: a plan model, or the row's model.
        let has = |f: Field| fields.iter().any(|(field, _)| *field == f);
        let picked = if has(Field::Plan) {
            staged.plan_model
        } else {
            has(Field::Model).then_some(staged.model)
        };
        let Some(model) = picked.filter(|m| m != "default" && m != "none") else {
            return self.save(row, &fields);
        };
        let app = staged.app;
        if st.probed.contains(&(app.name, model.clone())) {
            return self.save(row, &fields);
        }
        self.start_probe(Scope::Repo, row, fields, app, model);
    }

    /// The one-line prompt that tries model on app, off the screen thread:
    /// probed() takes its answer.
    fn start_probe(
        &mut self,
        scope: Scope,
        row: usize,
        fields: Vec<(Field, String)>,
        app: &'static App,
        model: String,
    ) {
        let repo = self.cfg.repo.clone();
        let tools = self.cfg.tools.clone();
        let argv = app::probe(app, &repo, &model);
        let (tx, result) = mpsc::channel();
        thread::spawn(move || {
            let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
            let _ = tx.send(tools.run(&repo, &argv));
        });
        self.settings.as_mut().unwrap().probe = Some(Probe {
            result,
            scope,
            row,
            fields,
            app: app.name,
            model,
        });
    }

    /// change for a scope: a label's override or its Extra review's saves
    /// into the label's entry.
    fn change_in(&mut self, scope: Scope, row: usize, fields: Vec<(Field, String)>) {
        match scope {
            Scope::Repo => self.change(row, fields),
            scope => self.change_override(scope, row, fields),
        }
    }

    /// A change to the open label's row field, or its Extra review's: staged
    /// over config.json read afresh and refused as the Stage pages refuse
    /// (a rule it breaks, a row a Stage could not start on); one that
    /// changes nothing is dropped; a named model is probed first.
    fn change_override(&mut self, scope: Scope, row: usize, fields: Vec<(Field, String)>) {
        let repo = self.cfg.repo.clone();
        let st = self.settings.as_mut().unwrap();
        let name = st.label.clone().unwrap_or_default();
        let key = ROWS[row].key;
        if fields.iter().all(|(f, v)| st.own(scope, row, *f) == *v) {
            return;
        }
        let row_now = match staged_override(&repo, &name, scope, key, &fields) {
            Ok((_, _, row_now)) => row_now,
            Err(err) => {
                st.note = Some((format!("Refused: {err}. Nothing changed."), RED));
                return;
            }
        };
        let named = fields
            .iter()
            .any(|(f, v)| *f == Field::Model && !v.is_empty());
        let model = row_now.model;
        if !named
            || model == "default"
            || model == "none"
            || st.probed.contains(&(row_now.app.name, model.clone()))
        {
            return self.save_override(scope, row, &fields);
        }
        self.start_probe(scope, row, fields, row_now.app, model);
    }

    /// The probe's answer, taken in poll(): the change saves, or the App's
    /// error shows in a red Notice modal and the old value stays.
    pub(super) fn probed(&mut self) {
        let Some(st) = &mut self.settings else {
            return;
        };
        let Some(Ok(result)) = st.probe.as_ref().map(|p| p.result.try_recv()) else {
            return;
        };
        let probe = st.probe.take().unwrap();
        match result {
            Ok(_) => {
                st.probed.push((probe.app, probe.model));
                match probe.scope {
                    Scope::Repo => self.save(probe.row, &probe.fields),
                    scope => self.save_override(scope, probe.row, &probe.fields),
                }
            }
            Err(err) => {
                let why = refusal(&err);
                let text = format!(
                    "{} refused {}: {why}. Nothing changed.",
                    probe.app, probe.model
                );
                self.notify(NoticeKind::Error, &text, None);
            }
        }
    }

    /// A change saved outside config.json's rows: the foot says it with
    /// tail, CONFIG, PICK or SKILL, and during a run RECENT and the log.
    fn done(&mut self, text: String, tail: &str) {
        let live = self.run.is_some();
        let st = self.settings.as_mut().unwrap();
        st.saved = Some(chrono::Local::now().format("%H:%M:%S").to_string());
        st.note = Some((format!("{text}{tail}"), GREEN));
        if live {
            self.say(&format!("config: {text}"));
        }
    }

    fn refused(&mut self, err: &str) {
        let st = self.settings.as_mut().unwrap();
        st.note = Some((format!("{err}. Nothing changed."), RED));
    }

    /// Your personal skills on or off, in the per-person config.json; the
    /// skills you have listed again.
    fn personal_to(&mut self, on: bool) {
        if let Err(err) = manifest::set_personal(&self.cfg.repo, on) {
            return self.refused(&format!("{}: {err}", on_call::CONFIG));
        }
        let found = found(&self.cfg.repo, &self.cfg.home, &*self.cfg.tools);
        self.reload_skills(found);
        self.settings.as_mut().unwrap().personal = on;
        let text = match on {
            true => "your personal skills on: the jobs can pick them",
            false => "your personal skills off: only Orqadence's, the repo's and built-in ones",
        };
        self.done(text.to_string(), &format!(", saved in {}", on_call::CONFIG));
    }

    /// TypeSafe on or off in config.json, which /config reads again.
    fn typesafe_to(&mut self, on: bool) {
        let repo = &self.cfg.repo;
        match app::set_typesafe(repo, on).and_then(|()| app::read_object(repo)) {
            Ok((_, doc)) => {
                self.settings.as_mut().unwrap().doc = doc;
                self.done(
                    format!("TypeSafe {}", if on { "on" } else { "off" }),
                    CONFIG,
                );
            }
            Err(err) => self.refused(&err),
        }
    }

    /// A floor typed: a number from 0 to 1 saves at once, nothing puts the
    /// default back, and the next Judgment reads it; anything else is
    /// refused, the text kept to mend.
    fn keep_floor(&mut self, floor: &'static Floor, text: String) {
        let st = self.settings.as_mut().unwrap();
        let value = match text.parse::<f64>() {
            _ if text.is_empty() => None,
            Ok(value) if (0.0..=1.0).contains(&value) => Some(value),
            _ => {
                let refused =
                    format!("Refused: {text} is not a number from 0 to 1. Nothing changed.");
                st.note = Some((refused, RED));
                st.typing = Some((Typing::Floor(floor), text));
                return;
            }
        };
        let repo = &self.cfg.repo;
        match app::set_floor(repo, floor, value).and_then(|()| app::read_object(repo)) {
            Ok((_, doc)) => {
                self.settings.as_mut().unwrap().doc = doc;
                let name = floor_name(floor);
                let text = match value {
                    Some(value) => format!("{name} {value:.2}"),
                    None => format!("{name} {:.2}, its default", floor.default),
                };
                self.done(text, CONFIG);
            }
            Err(err) => self.refused(&err),
        }
    }

    /// A whole number typed: one that keeps its rule saves at once, nothing
    /// puts the default back, and the live run reads it next; anything else
    /// is refused, the text kept to mend.
    fn keep_number(&mut self, n: &'static Number, text: String) {
        let st = self.settings.as_mut().unwrap();
        let value = match text.parse::<usize>() {
            _ if text.is_empty() => None,
            Ok(v) if v >= n.count.least => Some(v),
            _ => {
                let refused = format!(
                    "Refused: {text} is not {}. Nothing changed.",
                    n.count.rule()
                );
                st.note = Some((refused, RED));
                st.typing = Some((Typing::Number(n), text));
                return;
            }
        };
        let repo = &self.cfg.repo;
        match app::set_count(repo, n.count, value).and_then(|()| app::read_object(repo)) {
            Ok((_, doc)) => {
                self.settings.as_mut().unwrap().doc = doc;
                let text = match value {
                    Some(v) => format!("{}: {v}", n.name),
                    None => format!("{}: {}, its default", n.name, n.count.default),
                };
                self.done(text, CONFIG);
            }
            Err(err) => self.refused(&err),
        }
    }

    /// A switch on or off in config.json, which /config reads again. Agent
    /// merge going off with automatic Address PR comments is said on RECENT,
    /// run or no run: nobody turned it off by hand.
    fn switch_to(&mut self, switch: &'static Switch, on: bool) {
        let repo = &self.cfg.repo;
        match app::set_switch(repo, switch, on).and_then(|()| app::read_object(repo)) {
            Ok((_, doc)) => {
                let st = self.settings.as_mut().unwrap();
                let was_on = app::switch_in(&st.doc, &AGENT_MERGE);
                let went_off = was_on && !app::switch_in(&doc, &AGENT_MERGE);
                st.doc = doc;
                let name = Field::Switch(switch).name();
                self.done(format!("{name}: {}", if on { "on" } else { "off" }), CONFIG);
                if went_off && *switch != AGENT_MERGE {
                    let name = Field::Switch(&AGENT_MERGE).name();
                    self.say(&format!(
                        "config: {name}: off, as PR comments no longer open by themselves"
                    ));
                }
            }
            Err(err) => self.refused(&err),
        }
    }

    /// A review bot ticked or unticked in config.json's review_bots, which
    /// /config reads again; a list that cannot be read starts empty.
    fn tick_bot(&mut self, bot: &'static str) {
        let st = self.settings.as_ref().unwrap();
        let kept = app::review_bots_in(&st.doc).unwrap_or_default();
        let bots: Vec<_> = REVIEW_BOTS
            .into_iter()
            .filter(|b| kept.contains(b) != (*b == bot))
            .collect();
        let repo = &self.cfg.repo;
        match app::set_review_bots(repo, &bots).and_then(|()| app::read_object(repo)) {
            Ok((_, doc)) => {
                self.settings.as_mut().unwrap().doc = doc;
                let mut said = bots.join(", ");
                if bots.is_empty() {
                    said = "none".to_string();
                }
                self.done(format!("review bots: {said}"), CONFIG);
            }
            Err(err) => self.refused(&err),
        }
    }

    /// A key typed while there was none: kept as init keeps it, and
    /// TypeSafe on.
    // ponytail: a live run keeps the Config it started with, so its
    // Judgments get the key from the next run on.
    fn keep_key(&mut self, key: String) {
        match setup::keep_key(&self.cfg.repo, &key) {
            Ok(()) => {
                self.cfg.api_key = key;
                self.typesafe_to(true);
            }
            Err(err) => self.refused(&format!("{}: {err}", setup::KEY_FILE)),
        }
    }

    /// On call's settings as the file keeps them, changed and saved at once;
    /// the Screen's copy takes them for the next tick, but for a token
    /// MOSHI_WEBHOOK_TOKEN overrides.
    fn keep_on_call(&mut self, change: impl FnOnce(&mut OnCall), text: String) {
        let repo = &self.cfg.repo;
        // The file's own, so the environment's token is never written to it.
        let mut kept = on_call::load(repo, &|_| String::new());
        change(&mut kept);
        if let Err(err) = on_call::save(repo, &kept) {
            return self.refused(&format!("{}: {err}", on_call::CONFIG));
        }
        if !self.moshi_env {
            self.on_call.token = kept.token;
        }
        self.on_call.minutes = kept.minutes;
        self.done(text, &format!(", saved in {}", on_call::CONFIG));
    }

    /// The Moshi token typed: kept, or cleared by an empty entry.
    fn keep_token(&mut self, token: String) {
        let text = match token.is_empty() {
            true => "On call token cleared: On call off",
            false => "On call token set",
        };
        let text = match self.moshi_env {
            true => format!(
                "{text}; {} in the environment still wins",
                on_call::TOKEN_VAR
            ),
            false => text.to_string(),
        };
        let token = (!token.is_empty()).then_some(token);
        self.keep_on_call(|kept| kept.token = token, text);
    }

    /// On call's minutes typed: a whole number from 1 to u32::MAX saves at once,
    /// nothing puts the default back; anything else is refused, the text
    /// kept to mend.
    fn keep_minutes(&mut self, text: String) {
        let (minutes, said) = match text.parse::<u64>() {
            _ if text.is_empty() => (
                DEFAULT_MINUTES,
                format!("On call after {DEFAULT_MINUTES} minutes, its default"),
            ),
            Ok(n) if (1..=u32::MAX as u64).contains(&n) => {
                (n, format!("On call after {}", plural(n as usize, "minute")))
            }
            _ => {
                let st = self.settings.as_mut().unwrap();
                let refused = format!(
                    "Refused: {text} is not a whole number of at least 1. Nothing changed."
                );
                st.note = Some((refused, RED));
                st.typing = Some((Typing::Minutes, text));
                return;
            }
        };
        self.keep_on_call(|kept| kept.minutes = minutes, said);
    }

    /// Send a test push: the doorbell rung off the draw loop; refused with
    /// no token.
    fn test_push(&mut self) {
        let Some(token) = self.on_call.token.clone() else {
            let text = "Refused: no Moshi token set: Enter on token types one.";
            self.settings.as_mut().unwrap().note = Some((text.to_string(), RED));
            return;
        };
        let (doorbell, title) = (self.doorbell.clone(), format!("orqa · {}", self.folder));
        self.off_thread("sending a test push…".to_string(), move |_, _| {
            Done::Pushed(doorbell.ring(&token, &title, "test push from Orqadence"))
        });
    }

    /// Runs work on its own thread, then lists the skills you have there
    /// too; said in the foot until finished() takes its answer.
    fn off_thread(
        &mut self,
        text: String,
        work: impl FnOnce(&Path, &dyn Tools) -> Done + Send + 'static,
    ) {
        let (repo, home) = (self.cfg.repo.clone(), self.cfg.home.clone());
        let tools = self.cfg.tools.clone();
        let (tx, done) = mpsc::channel();
        thread::spawn(move || {
            let done = work(&repo, &*tools);
            let _ = tx.send((done, found(&repo, &home, &*tools)));
        });
        self.settings.as_mut().unwrap().busy = Some(Busy { text, done });
    }

    /// A pasted source: a bare name refused at once, the text kept to mend;
    /// anything else cloned.
    fn add_source(&mut self, text: String) {
        let st = self.settings.as_mut().unwrap();
        let source = match parse_source(&text) {
            Ok(source) => source,
            Err(err) => {
                st.note = Some((err, RED));
                st.typing = Some((Typing::Source, text));
                return;
            }
        };
        let said = format!("cloning {}…", short(&source.repo));
        self.off_thread(said, move |repo, tools| {
            let added = manifest::add(repo, tools, &text, None);
            Done::Added(text, added)
        });
    }

    /// The checklist's ticked skills not installed yet, each installed.
    // ponytail: a clone per skill ticked; one clone for them all when a
    // pack's size makes that slow.
    fn install_ticked(&mut self) {
        let st = self.settings.as_mut().unwrap();
        let listing = st.listing.as_ref().unwrap();
        let names: Vec<String> = listing
            .names
            .iter()
            .filter(|(_, installed, ticked)| *ticked && !installed)
            .map(|(name, _, _)| name.clone())
            .collect();
        if names.is_empty() {
            st.note = Some(("Space ticks a skill to install.".to_string(), MUTED));
            return;
        }
        let source = st.listing.take().unwrap().source;
        let said = format!("cloning {source} for {}…", names.join(", "));
        self.off_thread(said, move |repo, tools| {
            let added = names.into_iter().map(|name| {
                let added = manifest::add(repo, tools, &source, Some(&name));
                (name, added)
            });
            Done::Ticked(added.collect())
        });
    }

    /// A job's suggestion not installed: installed, then picked.
    fn install_for(&mut self, j: usize, name: &'static str, source: &'static str) {
        let from = parse_source(source).map_or(source.to_string(), |s| short(&s.repo).into());
        self.off_thread(
            format!("cloning {from} for {name}…"),
            move |repo, tools| Done::ForJob(j, manifest::add(repo, tools, source, Some(name))),
        );
    }

    /// A job's pick, saved in the Skill manifest read afresh; the pick it
    /// has changes nothing. Whether it saved.
    fn set_pick(&mut self, j: usize, name: &str) -> bool {
        let repo = &self.cfg.repo;
        let mut manifest = match Manifest::load(repo) {
            Ok(manifest) => manifest,
            Err(err) => {
                self.refused(&err);
                return false;
            }
        };
        if manifest.pick(JOBS[j].0) == name {
            return false;
        }
        manifest
            .picks
            .insert(JOBS[j].0.to_string(), name.to_string());
        if let Err(err) = manifest.save(repo) {
            self.refused(&err);
            return false;
        }
        self.settings.as_mut().unwrap().manifest = manifest;
        self.done(format!("{} picks {name}", job_said(j)), PICK);
        true
    }

    /// u on a skill, or U: fetched again off the screen thread. A Shipped
    /// skill is init's to update.
    fn update_skills(&mut self, one: Option<String>) {
        let st = self.settings.as_mut().unwrap();
        let said = match &one {
            Some(name) if st.manifest.skills[name].shipped => {
                let text = format!("{name} is a Shipped skill: orqa init updates it.");
                st.note = Some((text, RED));
                return;
            }
            Some(name) => format!(
                "fetching {} for {name}…",
                short(&st.manifest.skills[name].repo)
            ),
            None => "fetching every skill's source…".to_string(),
        };
        self.off_thread(said, move |repo, tools| {
            let failed = match &one {
                Some(name) => manifest::update(repo, tools, name).map(|()| Vec::new()),
                None => manifest::update_all(repo, tools),
            };
            Done::Updated(one, failed)
        });
    }

    /// d on a skill: asked first, naming the jobs it would leave on none. A
    /// Shipped skill is never removed.
    fn ask_remove(&mut self, name: String) {
        let st = self.settings.as_mut().unwrap();
        let skill = &st.manifest.skills[&name];
        if skill.shipped {
            let text = format!("{name} is a Shipped skill: it cannot be removed.");
            st.note = Some((text, RED));
            return;
        }
        let jobs = st.jobs_using(&name);
        let those = if jobs.len() == 1 {
            "that job"
        } else {
            "those jobs"
        };
        let text = match jobs.len() {
            0 => format!("Remove {name} ({})?", short(&skill.repo)),
            _ => format!(
                "{name} is the Delegate skill for {}. Remove it and set {those} to none?",
                jobs.join(", ")
            ),
        };
        st.confirm = Some((text, Confirm::Remove(name)));
    }

    /// A yes to removing a skill: no clone, so on the screen thread.
    fn remove_skill(&mut self, name: &str) {
        let jobs = self.settings.as_ref().unwrap().jobs_using(name);
        if let Err(err) = manifest::remove(&self.cfg.repo, name) {
            return self.refused(&err);
        }
        let found = found(&self.cfg.repo, &self.cfg.home, &*self.cfg.tools);
        self.reload_skills(found);
        let is = if jobs.len() == 1 { "is" } else { "are" };
        let text = match jobs.len() {
            0 => format!("removed {name}"),
            _ => format!("removed {name}; {} {is} none", jobs.join(", ")),
        };
        self.done(text, SKILL);
    }

    /// The Skill manifest read again after a change, with the skills you
    /// have found since.
    fn reload_skills(&mut self, found: Vec<(String, PathBuf)>) {
        let st = self.settings.as_mut().unwrap();
        if let Ok(manifest) = Manifest::load(&self.cfg.repo) {
            st.manifest = manifest;
        }
        st.found = found;
        if st.section == SKILLS_PAGE {
            st.setting = st.setting.min(st.manifest.skills.len() + SKILL_ROWS - 1);
        }
    }

    /// "installed tdd from mattpocock/skills @ abc1234".
    fn installed(&self, name: &str) -> String {
        let st = self.settings.as_ref().unwrap();
        match st.manifest.skills.get(name) {
            Some(skill) => format!(
                "installed {name} from {} @ {}",
                short(&skill.repo),
                short_commit(&skill.commit)
            ),
            None => format!("installed {name}"),
        }
    }

    /// A clone's answer, taken in poll(): the skills read again, and what it
    /// did said in the foot.
    pub(super) fn finished(&mut self) {
        let Some(st) = &mut self.settings else {
            return;
        };
        let (done, found) = match st.busy.as_ref().map(|b| b.done.try_recv()) {
            Some(Ok(got)) => got,
            Some(Err(TryRecvError::Disconnected)) => {
                st.busy = None;
                let text =
                    "The clone's thread died: /config shows what it installed when it opens again.";
                st.note = Some((text.to_string(), RED));
                return;
            }
            _ => return,
        };
        st.busy = None;
        let before = st.manifest.skills.clone();
        self.reload_skills(found);
        let st = self.settings.as_mut().unwrap();
        match done {
            Done::Added(_, Err(err)) | Done::ForJob(_, Err(err)) => self.refused(&err),
            Done::Added(_, Ok(Added::Installed(name))) => {
                let said = self.installed(&name);
                self.put_on_label(vec![name], said)
            }
            Done::Added(source, Ok(Added::Choose(names))) => {
                let repo = parse_source(&source).map(|s| s.repo).unwrap_or_default();
                let names = names
                    .into_iter()
                    .map(|name| {
                        let here = st
                            .manifest
                            .skills
                            .get(&format!("{}{name}", manifest::PREFIX))
                            .is_some_and(|s| s.repo == repo);
                        (name, here, here)
                    })
                    .collect();
                st.listing = Some(Listing {
                    source,
                    names,
                    cursor: 0,
                });
            }
            Done::ForJob(j, Ok(added)) => {
                let Added::Installed(name) = added else {
                    return;
                };
                let installed = self.installed(&name);
                if self.set_pick(j, &name) {
                    self.done(format!("{installed}; {} uses it", job_said(j)), SKILL);
                } else {
                    self.done(installed, SKILL);
                }
            }
            Done::Ticked(added) => {
                let mut said = Vec::new();
                let mut names = Vec::new();
                let mut failed = false;
                for (name, added) in added {
                    match added {
                        Ok(Added::Installed(name)) => {
                            said.push(self.installed(&name));
                            names.push(name);
                        }
                        Ok(Added::Choose(_)) => {}
                        Err(err) => {
                            failed = true;
                            said.push(format!("{name} not installed: {err}"));
                        }
                    }
                }
                match failed {
                    false => self.put_on_label(names, said.join("; ")),
                    true => self.refused(&said.join("; ")),
                }
            }
            Done::Pushed(Ok(())) => st.note = Some(("test push sent".to_string(), GREEN)),
            Done::Pushed(Err(err)) => st.note = Some((format!("test push failed: {err}"), RED)),
            Done::Updated(_, Err(err)) => self.refused(&err),
            Done::Updated(one, Ok(failed)) => {
                let changed: Vec<String> = st
                    .manifest
                    .skills
                    .iter()
                    .filter_map(|(name, skill)| {
                        let old = &before.get(name)?.commit;
                        (*old != skill.commit).then(|| {
                            format!(
                                "{name} {} → {}",
                                short_commit(old),
                                short_commit(&skill.commit)
                            )
                        })
                    })
                    .collect();
                let head = match (changed.is_empty(), &one) {
                    (false, _) => format!("updated {}", changed.join(", ")),
                    (true, Some(name)) => format!("{name} is up to date"),
                    (true, None) => "every skill is up to date".to_string(),
                };
                let said = std::iter::once(head)
                    .chain(
                        failed
                            .iter()
                            .map(|(name, why)| format!("{name} not updated: {why}")),
                    )
                    .collect::<Vec<_>>()
                    .join("; ");
                match (failed.is_empty(), changed.is_empty()) {
                    (true, true) => st.note = Some((said, GREEN)),
                    (true, false) => self.done(said, SKILL),
                    _ => self.refused(&said),
                }
            }
        }
    }

    /// A change to config.json's labels, read afresh and written whole; the
    /// foot says it, uncommitted. A labels key that is not an object
    /// refuses. Whether it saved.
    fn save_label(
        &mut self,
        change: impl FnOnce(&mut serde_json::Map<String, Value>) -> Result<(), String>,
        text: String,
    ) -> bool {
        let saved = app::read_object(&self.cfg.repo).and_then(|(path, mut doc)| {
            if doc["labels"].is_null() {
                doc["labels"] = json!({});
            }
            let labels = doc["labels"]
                .as_object_mut()
                .ok_or_else(|| "labels in config.json is not an object".to_string())?;
            change(labels)?;
            app::write(&path, &doc).map(|()| doc)
        });
        self.label_saved(saved, text)
    }

    /// config.json as a label change wrote it, or why it did not.
    fn label_saved(&mut self, saved: Result<Value, String>, text: String) -> bool {
        match saved {
            Ok(doc) => {
                self.settings.as_mut().unwrap().doc = doc;
                self.done(text, CONFIG);
                true
            }
            Err(err) => {
                self.refused(&err);
                false
            }
        }
    }

    /// a's name typed: an invalid one or a clash is refused, the text kept
    /// to mend; else the entry is written as an area label and its page
    /// opens.
    fn add_label(&mut self, typed: String) {
        let st = self.settings.as_mut().unwrap();
        let name = match st.valid_label(&typed) {
            Ok(name) => name,
            Err(err) => {
                st.note = Some((err, RED));
                st.typing = Some((Typing::LabelName, typed));
                return;
            }
        };
        let text = format!("added orqa:{name}, an area label");
        let added = name.clone();
        let saved = self.save_label(
            move |labels| {
                if labels.contains_key(&added) {
                    return Err(format!("orqa:{added} is there already"));
                }
                labels.insert(added, json!({"kind": "area"}));
                Ok(())
            },
            text,
        );
        if saved {
            let st = self.settings.as_mut().unwrap();
            st.label = Some(name);
            st.setting = 0;
        }
    }

    /// r's name typed: the entry moves whole under the new name, its
    /// pr_template mapping with it; the same name changes nothing.
    fn rename_label(&mut self, old: String, typed: String) {
        let st = self.settings.as_mut().unwrap();
        if label_name(&typed) == old {
            st.note = Some(("Same name: nothing changed.".to_string(), MUTED));
            return;
        }
        let name = match st.valid_label(&typed) {
            Ok(name) => name,
            Err(err) => {
                st.note = Some((err, RED));
                st.typing = Some((Typing::Rename(old), typed));
                return;
            }
        };
        let text = format!("renamed orqa:{old} to orqa:{name}");
        let new = name.clone();
        self.save_label(
            move |labels| {
                if labels.contains_key(&new) {
                    return Err(format!("orqa:{new} is there already"));
                }
                let entry = labels
                    .remove(&old)
                    .ok_or_else(|| format!("config.json has no label orqa:{old}"))?;
                labels.insert(new, entry);
                Ok(())
            },
            text,
        );
        self.labels_cursor(&name);
    }

    /// A yes to deleting a label: its entry goes, its PR template file
    /// stays.
    fn delete_label(&mut self, name: &str) {
        let gone = name.to_string();
        self.save_label(
            move |labels| {
                labels.remove(&gone);
                Ok(())
            },
            format!("deleted orqa:{name}"),
        );
        self.labels_cursor(name);
    }

    /// The list's cursor on name, or clamped to the list.
    fn labels_cursor(&mut self, name: &str) {
        let st = self.settings.as_mut().unwrap();
        let names = st.label_names();
        st.setting = names
            .iter()
            .position(|n| n == name)
            .unwrap_or(st.setting.min(names.len().saturating_sub(1)));
    }

    /// Enter or Space on the kind row: area to modifier and back.
    fn toggle_kind(&mut self, name: &str) {
        let st = self.settings.as_ref().unwrap();
        let (kind, a) = match st.label_of(name).map(|l| l.kind).as_deref() {
            Ok("area") => ("modifier", "a modifier"),
            _ => ("area", "an area"),
        };
        let label = name.to_string();
        self.save_label(
            move |labels| {
                label_entry(labels, &label)?["kind"] = json!(kind);
                Ok(())
            },
            format!("orqa:{name} is {a} label"),
        );
    }

    /// A skill picked on the open label's list: on the label, or off it
    /// again; the list stays open.
    fn toggle_skill(&mut self, pick: Pick, skill: &str) {
        let st = self.settings.as_ref().unwrap();
        let name = st.label.clone().unwrap_or_default();
        let on = st
            .label_of(&name)
            .is_ok_and(|l| l.skills.iter().any(|s| s == skill));
        let text = format!("orqa:{name} {} {skill}", if on { "drops" } else { "takes" });
        let skill = skill.to_string();
        self.save_label(
            move |labels| {
                put_skills(labels, &name, |skills| match on {
                    true => skills.retain(|s| *s != skill),
                    false => skills.push(skill),
                })
            },
            text,
        );
        self.settings.as_mut().unwrap().pick = Some(pick);
    }

    /// Skills installed: put on the open label when one is, and said; said
    /// alone otherwise.
    fn put_on_label(&mut self, names: Vec<String>, said: String) {
        let Some(name) = self.settings.as_ref().unwrap().label.clone() else {
            return self.done(said, SKILL);
        };
        let it = if names.len() == 1 { "it" } else { "them" };
        let text = format!("{said}; orqa:{name} takes {it}");
        self.save_label(
            move |labels| {
                put_skills(labels, &name, |skills| {
                    for name in names {
                        if !skills.contains(&name) {
                            skills.push(name);
                        }
                    }
                })
            },
            text,
        );
    }

    /// The guidance line typed: saved as it is, nothing clearing it.
    fn keep_guidance(&mut self, text: String) {
        let st = self.settings.as_ref().unwrap();
        let name = st.label.clone().unwrap_or_default();
        let said = match text.is_empty() {
            true => format!("orqa:{name} guidance cleared"),
            false => format!("orqa:{name} guidance set"),
        };
        self.save_label(
            move |labels| {
                label_entry(labels, &name)?["guidance"] = json!(text);
                Ok(())
            },
            said,
        );
    }

    /// The change into the open label's entry: "orqa:be implement model
    /// opus", or "is the repo's" for an emptied field.
    fn save_override(&mut self, scope: Scope, row: usize, fields: &[(Field, String)]) {
        let name = self.open_label();
        let (what, falls) = match scope {
            Scope::Extra => ("extra review".to_string(), "the Review's"),
            _ => (ROWS[row].key.replace('_', " "), "the repo's"),
        };
        let parts: Vec<String> = fields
            .iter()
            .map(|(f, v)| match v.is_empty() {
                true => format!("{} is {falls}", f.name()),
                false => format!("{} {v}", f.name()),
            })
            .collect();
        let text = format!("orqa:{name} {what} {}", parts.join(", "));
        // staged again: config.json may have changed during a probe
        let saved = staged_override(&self.cfg.repo, &name, scope, ROWS[row].key, fields)
            .and_then(|(path, doc, _)| app::write(&path, &doc).map(|()| doc));
        self.label_saved(saved, text);
    }

    /// The name of the label open on the Labels page; empty with none.
    fn open_label(&self) -> String {
        self.settings
            .as_ref()
            .unwrap()
            .label
            .clone()
            .unwrap_or_default()
    }

    /// Enter (or Space, when it toggles) on an item of the open label's
    /// page.
    fn label_item(&mut self, name: &str, item: LabelItem, enter: bool) {
        let st = self.settings.as_mut().unwrap();
        let (scope, row, field) = match item {
            LabelItem::Kind => return self.toggle_kind(name),
            LabelItem::Position => return self.cycle_position(name),
            LabelItem::Debate => return self.toggle_debate(name),
            // the rest are Enter's alone
            _ if !enter => return,
            LabelItem::Skills => (Scope::Repo, 0, Field::Skills),
            LabelItem::Template => (Scope::Repo, 0, Field::Template),
            LabelItem::ExtraSkill => (Scope::Repo, 0, Field::ExtraSkill),
            LabelItem::Extra(field) => (Scope::Extra, REVIEW_ROW, field),
            LabelItem::Row(row, field) => (Scope::Label, row, field),
            LabelItem::Guidance => {
                let text = st.label_of(name).map(|l| l.guidance).unwrap_or_default();
                st.typing = Some((Typing::Guidance, text));
                return;
            }
        };
        let app = app(&value(&st.view(scope), row, Field::App));
        match app {
            Some(app) if field == Field::Effort && app.effort.is_empty() => {
                let text = format!("{} has no effort flag.", app.name);
                st.note = Some((text, MUTED));
            }
            _ => self.open_pick(row, field, None, scope),
        }
    }

    /// Enter or Space on position: every Round, the first Round only,
    /// before the PR, and round again.
    fn cycle_position(&mut self, name: &str) {
        let st = self.settings.as_ref().unwrap();
        let now = st
            .label_of(name)
            .map(|l| l.extra_review.position)
            .unwrap_or_default();
        let next = match now.as_str() {
            "first" => "before_pr",
            "before_pr" => "every",
            _ => "first",
        };
        let text = format!("orqa:{name} Extra review runs {}", position_name(next));
        self.put_extra(name, text, |extra| extra["position"] = json!(next));
    }

    /// Enter or Space on Debate: its Findings join it, or go straight to
    /// the Fix.
    fn toggle_debate(&mut self, name: &str) {
        let st = self.settings.as_ref().unwrap();
        let on = !st.label_of(name).is_ok_and(|l| l.extra_review.debate);
        let text = match on {
            true => format!("orqa:{name} Extra review Findings join the Debate"),
            false => {
                format!("orqa:{name} Extra review Findings go straight to the Fix, not debated")
            }
        };
        self.put_extra(name, text, |extra| extra["debate"] = json!(on));
    }

    /// An Extra review skill picked: none clears it, and with it the Extra
    /// review.
    fn pick_extra_skill(&mut self, skill: &str) {
        let name = self.open_label();
        let text = match skill.is_empty() {
            true => format!("orqa:{name} has no Extra review"),
            false => format!("orqa:{name} Extra review is {skill}"),
        };
        let skill = skill.to_string();
        self.put_extra(&name, text, move |extra| match skill.is_empty() {
            true => _ = extra.as_object_mut().unwrap().remove("skill"),
            false => extra["skill"] = json!(skill),
        });
    }

    /// A change to the label's Extra review entry, saved at once.
    fn put_extra(&mut self, name: &str, text: String, change: impl FnOnce(&mut Value)) {
        let name = name.to_string();
        self.save_label(
            move |labels| {
                change(extra_mut(label_entry(labels, &name)?));
                Ok(())
            },
            text,
        );
    }

    /// A PR template picked: a file of the directory, or default, which
    /// takes the mapping off the entry.
    fn pick_template(&mut self, file: &str) {
        let name = self.open_label();
        let text = match file.is_empty() {
            true => format!("orqa:{name} uses the default PR template"),
            false => format!("orqa:{name} uses the PR template {file}"),
        };
        self.map_template(file, text);
    }

    /// The entry's pr_template set to file; empty removes it.
    fn map_template(&mut self, file: &str, text: String) {
        let name = self.open_label();
        let file = file.to_string();
        self.save_label(
            move |labels| {
                let entry = label_entry(labels, &name)?;
                match file.is_empty() {
                    true => _ = entry.as_object_mut().unwrap().remove("pr_template"),
                    false => entry["pr_template"] = json!(file),
                }
                Ok(())
            },
            text,
        );
    }

    /// 'new…' on the PR template list: a shipped label's gets its shipped
    /// section at once, any other's asks for a heading. Never over a file
    /// that is there.
    fn new_template(&mut self) {
        let st = self.settings.as_mut().unwrap();
        let name = st.label.clone().unwrap_or_default();
        let at = format!("{}/{name}.md", setup::TEMPLATE_DIR);
        if name.contains(['/', '\\']) {
            return self.refused(&format!("orqa:{name} cannot name a file: rename it first"));
        }
        if self.cfg.repo.join(&at).exists() {
            return self.refused(&format!("{at} is there already: pick it from the list"));
        }
        let shipped = setup::LABELS.iter().find(|l| l.name == name);
        match shipped.filter(|l| !l.pr_section.is_empty()) {
            Some(label) => self.write_template(label.pr_section),
            None => st.typing = Some((Typing::Heading, String::new())),
        }
    }

    /// The heading typed: a section of it after the default template.
    fn keep_heading(&mut self, typed: &str) {
        match typed.trim_start_matches('#').trim() {
            "" => self.refused("No heading"),
            heading => self.write_template(&format!("## {heading}\n")),
        }
    }

    /// <label>.md in the template directory: the default template (the
    /// repo's, else Orqadence's) and the section, mapped on the label.
    fn write_template(&mut self, section: &str) {
        let repo = &self.cfg.repo;
        let name = self.open_label();
        let file = format!("{name}.md");
        let frame = setup::default_template(repo)
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_else(|| setup::PR_TEMPLATE.to_string());
        let dir = repo.join(setup::TEMPLATE_DIR);
        // create_new: a file made since the check in new_template is not overwritten
        let written = std::fs::create_dir_all(&dir).and_then(|()| {
            let mut out = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(dir.join(&file))?;
            std::io::Write::write_all(&mut out, setup::with_section(&frame, section).as_bytes())
        });
        if let Err(err) = written {
            return self.refused(&format!("{}/{file}: {err}", setup::TEMPLATE_DIR));
        }
        self.settings.as_mut().unwrap().templates = setup::template_files(&dir);
        let text = format!("wrote {}/{file}; orqa:{name} uses it", setup::TEMPLATE_DIR);
        self.map_template(&file, text);
    }

    /// Writes the change into config.json, read afresh; during a run RECENT
    /// and the log say what changed.
    fn save(&mut self, row: usize, fields: &[(Field, String)]) {
        let saved = staged(&self.cfg.repo, ROWS[row].key, fields)
            .and_then(|(path, old, doc, _)| app::write(&path, &doc).map(|()| (old, doc)));
        let live = self.run.is_some();
        let st = self.settings.as_mut().unwrap();
        let (old, doc) = match saved {
            Ok(docs) => docs,
            Err(err) => {
                st.note = Some((format!("{err}. Nothing changed."), RED));
                return;
            }
        };
        // as it was on disk, a hand edit since /config opened included
        let (old, new) = (said(&old, row), said(&doc, row));
        st.doc = doc;
        st.saved = Some(chrono::Local::now().format("%H:%M:%S").to_string());
        let name = ROWS[row].name;
        let text = match live {
            true => format!(
                "saved: {name} {new}, uncommitted. Stages that start from now use it; running ones keep theirs."
            ),
            false => format!("saved: {name} {new}, uncommitted in .orqadence/config.json"),
        };
        st.note = Some((text, GREEN));
        if live {
            self.say(&format!("config: {name} {old} → {new}"));
        }
    }
}
