//! Stage results: what a Stage's session leaves behind, interpreted once for
//! the Pipeline, and the prompt a Stage's session is started with.

use std::fs;
use std::path::{Path, PathBuf};

/// The accepted content of a Stage result. Session liveness is a separate
/// part of Stage completion.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct StageResult {
    /// A review's Findings, each line as written: "- (severity) path — what".
    pub(crate) found: Vec<String>,
    pub(crate) fixes: Vec<String>,
    pub(crate) skips: Vec<String>,
    pub(crate) pr: String,
    /// The Review did not run: its App was Limited and the answer was to
    /// open the PR unreviewed. Why, for the Fix's Input.
    pub(crate) unreviewed: String,
    /// The Release's new version, as its VERSION: line wrote it: v1.5.0.
    pub(crate) version: String,
    /// A charting result's Map, as its MAP: line names it.
    pub(crate) map: String,
    /// A Waypoint session's Waypoint, as its WAYPOINT: line names it.
    pub(crate) waypoint: String,
    /// A charting result's Tickets, as its TICKETS: line lists them.
    pub(crate) tickets: Vec<String>,
    /// A brainstorm-epic result's Epics, in build order, as its EPICS: line
    /// lists them.
    pub(crate) epics: Vec<String>,
    /// Each LABEL: line's text: a label the session proposes.
    pub(crate) labels: Vec<String>,
}

/// The Pipeline context needed to accept a result. The default requires only
/// STATUS: done.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ResultRequirements {
    /// A Verdict must settle at least this many Findings.
    pub(crate) review_findings: usize,
    /// The final Fix must identify its opened PR.
    pub(crate) require_pr: bool,
    /// The Release must name its new version.
    pub(crate) require_version: bool,
}

/// The Wake reasons a result file gives (docs/design/events.md).
const NO_RESULT: &str = "went idle without a result";
const NOT_STATUS: &str = "wrote a result file whose first line is not STATUS:";
/// STATUS: question: neither done nor a Wake (read_question).
pub(crate) const ASKED: &str = "asked a question";
/// STATUS: manual: Manual work its session waits on, neither done nor a
/// Wake (read_manual).
pub(crate) const MANUAL: &str = "filed Manual work it waits on";
/// STATUS: plan: a two-step Plan ready, neither done nor a Wake (plan.rs).
pub(crate) const PLANNED: &str = "wrote its plan";

/// Interprets and accepts result contents for live completion, resume, and
/// late completion alike. A nonempty reason means the result is not accepted;
/// the caller decides whether to start a session, Wake, or keep waiting.
pub(crate) fn read_stage_result(path: &Path, want: ResultRequirements) -> (StageResult, String) {
    let rejected = |reason: &str| (StageResult::default(), reason.to_string());
    let Ok(body) = fs::read_to_string(path) else {
        return rejected(NO_RESULT);
    };
    let first = body.lines().next().unwrap_or("").trim();
    let Some(value) = first.strip_prefix("STATUS:") else {
        return rejected(NOT_STATUS);
    };
    match value.trim().to_lowercase().as_str() {
        "failed" => return rejected("session reported failure"),
        "question" => return rejected(ASKED),
        "manual" => return rejected(MANUAL),
        "plan" => return rejected(PLANNED),
        "done" => {}
        _ => return rejected(NOT_STATUS),
    }

    let mut result = StageResult::default();
    let mut pr_pending = false; // "PR:" seen with nothing after it on its line
    for line in body.lines() {
        if pr_pending && !line.trim().is_empty() {
            result.pr = line.split_whitespace().next().unwrap_or("").to_string();
            pr_pending = false;
        }
        if line.starts_with("- (") {
            result.found.push(line.trim().to_string());
        }
        let lower = line.to_lowercase();
        if lower.starts_with("- [fix]") {
            result.fixes.push(line.trim().to_string());
        } else if lower.starts_with("- [skip]") {
            result.skips.push(line.trim().to_string());
        }
        if let Some(why) = line.strip_prefix("UNREVIEWED:") {
            result.unreviewed = why.trim().to_string();
        }
        // The first VERSION: line, as for PR: the free text after it may
        // start a line the same way.
        if let Some(version) = line
            .strip_prefix("VERSION:")
            .filter(|_| result.version.is_empty())
        {
            result.version = version.trim().to_string();
        }
        if let Some(map) = line.strip_prefix("MAP:").filter(|_| result.map.is_empty()) {
            result.map = map.trim().to_string();
        }
        if let Some(id) = line
            .strip_prefix("WAYPOINT:")
            .filter(|_| result.waypoint.is_empty())
        {
            result.waypoint = id.trim().to_string();
        }
        if let Some(ids) = line
            .strip_prefix("TICKETS:")
            .filter(|_| result.tickets.is_empty())
        {
            result.tickets = ids.split_whitespace().map(String::from).collect();
        }
        if let Some(ids) = line
            .strip_prefix("EPICS:")
            .filter(|_| result.epics.is_empty())
        {
            result.epics = ids.split_whitespace().map(String::from).collect();
        }
        if let Some(label) = line.strip_prefix("LABEL:") {
            result.labels.push(label.trim().to_string());
        }
        // As ^PR:\s*(\S+): the whitespace may cross blank lines.
        if let Some(rest) = line.strip_prefix("PR:").filter(|_| result.pr.is_empty()) {
            match rest.split_whitespace().next() {
                Some(pr) => result.pr = pr.to_string(),
                None => pr_pending = true,
            }
        }
    }
    // The Moderator can add audit Findings, so more settled items are valid.
    let settled = result.fixes.len() + result.skips.len();
    if settled < want.review_findings {
        return rejected(&format!(
            "Verdict settles {settled} of the Review's {} Findings",
            want.review_findings
        ));
    }
    if want.require_pr && result.pr.is_empty() {
        return rejected("finished without a PR link");
    }
    if want.require_version && result.version.is_empty() {
        return rejected("finished without a VERSION line");
    }
    (result, String::new())
}

/// A Stage's own question, its result file's first line STATUS: question:
/// the question text as written, and its options, the last block of lines
/// that start with "- " (a hunk in the question keeps its removed lines).
pub(crate) fn read_question(path: &Path) -> Option<(String, Vec<String>)> {
    let body = fs::read_to_string(path).ok()?;
    let mut lines = body.trim_end().lines();
    let first = lines.next()?.trim().strip_prefix("STATUS:")?;
    if !first.trim().eq_ignore_ascii_case("question") {
        return None;
    }
    let lines: Vec<&str> = lines.collect();
    let from = lines
        .iter()
        .rposition(|line| !line.starts_with("- "))
        .map_or(0, |i| i + 1);
    let options = lines[from..].iter().map(|o| o[2..].trim().to_string());
    let text = lines[..from].join("\n");
    Some((text.trim_matches('\n').to_string(), options.collect()))
}

/// Manual work a Stage's session waits on, its result file's first line
/// STATUS: manual: the item's folder, as the next line names it; empty
/// when no line does.
pub(crate) fn read_manual(path: &Path) -> Option<PathBuf> {
    let body = fs::read_to_string(path).ok()?;
    let mut lines = body.lines().map(str::trim);
    let first = lines.next()?.strip_prefix("STATUS:")?;
    if !first.trim().eq_ignore_ascii_case("manual") {
        return None;
    }
    Some(
        lines
            .find(|line| !line.is_empty())
            .unwrap_or_default()
            .into(),
    )
}

/// The text a Stage's session is prompted with: the Stage skill's body
/// followed by this run's inputs. The text is passed whole because Codex does
/// not load Claude skills and a worktree may not contain them.
pub(crate) fn stage_prompt(skill: &str, inputs: &[(&str, &str)]) -> String {
    let body = skill
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .map_or(skill, |(_, body)| body);
    let mut prompt = format!("{}\n\n## Inputs\n\n", body.trim());
    for (name, value) in inputs {
        prompt += &format!("- {name}: {value}\n");
    }
    prompt
}
