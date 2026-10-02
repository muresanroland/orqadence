//! Manual work: an item a Stage's session filed for the user, one folder
//! `manual-work/<n>/` in its Run directory: manual-work.md always, and
//! wizard.sh or prompt.md as the task needs. The session writes every file;
//! this reads them, and marks an item done.

use std::fs;
use std::path::{Path, PathBuf};

use crate::tools::Tools;

/// One item, as its folder holds it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Item {
    pub(crate) folder: PathBuf,
    /// manual-work.md's first line: Ticket: <id> · Stage: <stage> · Blocks: yes|no.
    pub(crate) ticket: String,
    pub(crate) stage: String,
    pub(crate) blocks: bool,
    /// Its sections, as written; empty when one is not there.
    pub(crate) what: String,
    pub(crate) why: String,
    pub(crate) how: String,
    pub(crate) report_back: String,
    /// wizard.sh is there.
    pub(crate) wizard: bool,
    /// prompt.md is there.
    pub(crate) prompt: bool,
}

/// Reads the item in `folder`: an error when its manual-work.md is missing
/// or its first line is not Ticket, Stage and Blocks.
pub(crate) fn read(folder: &Path) -> Result<Item, String> {
    let file = folder.join("manual-work.md");
    let body = fs::read_to_string(&file).map_err(|err| format!("{}: {err}", file.display()))?;
    let mut lines = body.lines();
    let first = lines.next().unwrap_or_default();
    let field = |name: &str| {
        first
            .split(" · ")
            .find_map(|part| part.trim().strip_prefix(name)?.strip_prefix(':'))
            .map(|value| value.trim().to_string())
    };
    let (Some(ticket), Some(stage), Some(blocks)) =
        (field("Ticket"), field("Stage"), field("Blocks"))
    else {
        return Err(format!(
            "{}: its first line is not Ticket: <id> · Stage: <stage> · Blocks: yes|no",
            file.display()
        ));
    };
    let mut sections: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in lines {
        match (line.strip_prefix("## "), sections.last_mut()) {
            (Some(heading), _) => sections.push((heading.trim(), Vec::new())),
            (None, Some((_, text))) => text.push(line),
            (None, None) => {}
        }
    }
    let section = |name: &str| {
        sections
            .iter()
            .find(|(heading, _)| heading.eq_ignore_ascii_case(name))
            .map_or(String::new(), |(_, text)| {
                text.join("\n").trim().to_string()
            })
    };
    Ok(Item {
        folder: folder.to_path_buf(),
        ticket,
        stage,
        blocks: blocks.eq_ignore_ascii_case("yes"),
        what: section("What"),
        why: section("Why"),
        how: section("How"),
        report_back: section("Report back"),
        wizard: folder.join("wizard.sh").is_file(),
        prompt: folder.join("prompt.md").is_file(),
    })
}

/// A Run directory's item folders, manual-work/<n>/, in number order.
#[allow(dead_code)] // its callers come with the notice and /manual-work (harness-a19.3, .4)
pub(crate) fn list(run_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(run_dir.join("manual-work")) else {
        return Vec::new();
    };
    let mut items: Vec<(u64, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| Some((entry.file_name().to_str()?.parse().ok()?, entry.path())))
        .collect();
    items.sort();
    items.into_iter().map(|(_, path)| path).collect()
}

/// An item's number, its folder's name: the n of manual-work/<n>/.
pub(crate) fn number(folder: &Path) -> String {
    folder
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

/// The prompt that tells the waiting session its item is done, with the
/// user's Report back `facts`, empty for Done alone.
pub(crate) fn done_prompt(folder: &Path, facts: &str) -> String {
    format!("Manual work {} done: {facts}", number(folder))
}

/// Marks `item` done: a bd comment on the id whose Run directory holds it (a
/// Ticket's), with its What and the user's `facts`, then its folder deleted.
/// A comment bd refuses leaves the folder.
pub(crate) fn done(tools: &dyn Tools, repo: &Path, item: &Item, facts: &str) -> Result<(), String> {
    // manual-work/<n>/ in the Run directory, which is named by its id
    let run_dir = item.folder.parent().and_then(Path::parent);
    let id = run_dir.and_then(Path::file_name).unwrap_or_default();
    let id = id.to_string_lossy();
    let n = number(&item.folder);
    let mut comment = format!("Manual work {n} done: {}", item.what);
    if !facts.is_empty() {
        comment += &format!("\n\nFacts: {facts}");
    }
    tools
        .run(repo, &["bd", "comments", "add", &id, &comment])
        .map_err(|err| err.to_string())?;
    fs::remove_dir_all(&item.folder).map_err(|err| format!("{}: {err}", item.folder.display()))
}
